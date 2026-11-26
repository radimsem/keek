use super::*;
use crate::window_seat::WindowSeat;
/// What the connection that sent an open_session offers the open.
pub(super) struct Caller {
    pub(super) opener: Option<watch::Receiver<()>>,
    pub(super) events: Option<mpsc::Sender<Event>>,
}
impl Registry {
    pub(super) fn begin_open(
        &mut self,
        project: String,
        worktree: PathBuf,
        goal: String,
        checks: Vec<String>,
        reply: oneshot::Sender<Response>,
        caller: Caller,
    ) {
        let Caller { mut opener, events } = caller;
        let refusal = match self.lifecycle {
            Lifecycle::ShuttingDown => {
                Some("keek is ShuttingDown. Start keek again to open an Agent Session.".to_owned())
            }
            Lifecycle::Running if self.len() >= self.dependencies.cap => Some(format!(
                "Agent Session cap {} reached. Close an open Agent Session first.",
                self.dependencies.cap
            )),
            Lifecycle::Running => None,
        };
        if let Some(message) = refusal {
            let _ = reply.send(Response::Refused { message });
            return;
        }
        let host = self.dependencies.host.clone();
        let runner = Arc::clone(&self.dependencies.runner);
        let sessions_dir = self.dependencies.sessions_dir.clone();
        let projects_config = self.dependencies.projects_config.clone();
        let mut catalog = self.catalog.clone();
        self.preparing.spawn(async move {
            let preparing = async {
                host.validate()?;
                project
                    .parse::<SessionName>()
                    .map_err(|e| SessionError::Refused(e.to_string()))?;
                let (profile, mut warnings) = crate::project::read_project_profile(&project, &projects_config).await?;
                let credential_path = projects_config.join(format!("{project}.env"));
                let (credentials, warning) = tokio::task::spawn_blocking(move || crate::credentials::Credentials::read(&credential_path)).await.map_err(|e| SessionError::Refused(e.to_string()))?.map_err(|e| SessionError::Refused(e.to_string()))?;
                let test_files = crate::test_files::TestFiles::read(&projects_config, &project)
                    .await.map_err(|e| SessionError::Refused(e.to_string()))?;
                warnings.extend(warning);
                let progress = crate::open_progress::OpenProgress::new(events, credentials.clone());
                let goal = credentials.mask(&goal);
                let checks = checks.iter().map(|text| credentials.mask(text)).collect();
                let catalog = wait_catalog(&mut catalog)
                    .await
                    .map_err(SessionError::Refused)?;
                let (worktree, revision) = checked_worktree(runner.as_ref(), sessions_dir, worktree).await?;
                Ok(Opening {
                    test_files,
                    credentials,
                    project,
                    worktree,
                    goal,
                    checks,
                    catalog,
                    revision,
                    profile, warnings,
                    progress,
                })
            };
            let result = tokio::select! { biased;
                () = async { match &mut opener { Some(receiver) => { let _ = receiver.changed().await; }, None => std::future::pending().await } } => Err(SessionError::Refused("The coding agent disconnected during setup. Open a new Agent Session.".into())),
                result = preparing => result,
            };
            Prepared { result, reply, opener }
        });
    }
    pub(super) async fn claim(
        &mut self,
        opening: Opening,
        opener: Option<watch::Receiver<()>>,
    ) -> Result<(Actor, Inbox), SessionError> {
        let Opening {
            test_files,
            credentials,
            project,
            worktree,
            goal,
            checks,
            catalog,
            revision,
            profile,
            warnings,
            progress,
        } = opening;
        match self.lifecycle {
            Lifecycle::Running => {}
            Lifecycle::ShuttingDown => {
                return Err(SessionError::Refused(
                    "keek is ShuttingDown. Start keek again to open an Agent Session.".into(),
                ));
            }
        }
        if self
            .sessions
            .values()
            .any(|entry| entry.worktree == worktree)
        {
            return Err(SessionError::Refused(format!(
                "An Agent Session is already open for {}. Close it first.",
                worktree.display()
            )));
        }
        let arrival = self.places.vacancy().ok_or_else(occupied)?;
        let workspace = self
            .dependencies
            .compositor
            .workspace(self.dependencies.cap)
            .await
            .map_err(|error| SessionError::Refused(error.to_string()))?;
        let slot = self
            .free_slot(arrival.replaced.as_ref())
            .ok_or_else(occupied)?;
        let (placement, spot) =
            self.opening_placement(&workspace, arrival.place, profile.window)?;
        let window = WindowSeat::new(placement.initial);
        let window_size = (
            placement.geometry.window.width,
            placement.geometry.window.height,
        );
        let opened_at = self.dependencies.clock.now();
        let started = self.dependencies.clock.local_time();
        let time = started.format("%H-%M-%S").to_string();
        let naming_project = project.clone();
        let sessions = self.dependencies.sessions_dir.clone();
        let used: Vec<_> = self
            .sessions
            .keys()
            .chain(self.cards.keys())
            .map(ToString::to_string)
            .collect();
        let name = tokio::task::spawn_blocking(move || {
            choose_name(&naming_project, &time, |name| {
                used.iter().any(|used| used == name) || sessions.join(name).exists()
            })
        })
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))??;
        let journal = crate::session_journal::Handle::create(
            &self.dependencies.journals_dir,
            crate::session_journal::Journal {
                project: project.clone(),
                name: name.clone(),
                started: started.to_rfc3339(),
                revision: revision.clone(),
                goal: goal.clone(),
                checks: crate::checklist::Checklist::new(checks.clone())
                    .checks()
                    .to_vec(),
                timeline: crate::timeline::SessionTimeline::new(crate::session::VIDEO_LEAD),
                takeovers: vec![],
                help_requests: vec![],
                credentials_hash: credentials.fingerprint(),
                profile: crate::report::ProfileSummary::new(
                    &profile,
                    &crate::toolchain::Toolchains::default(),
                )
                .with_test_files(&test_files.list)
                .masked(&credentials),
            },
        )
        .await?;
        let folder = SessionFolder::new(&self.dependencies.sessions_dir, &name);
        let root = folder.root().to_owned();
        let record = serde_json::json!({"goal":goal,"checks":checks,"revision":revision,"worktree":worktree});
        let folder_result = tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&root)?;
            std::fs::write(
                root.join("session.json"),
                serde_json::to_vec_pretty(&record)?,
            )
        })
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))
        .and_then(|result| result.map_err(Into::into));
        if let Err(error) = folder_result {
            journal.remove().await;
            return Err(error);
        }
        if self.sessions.is_empty()
            && let Err(error) = self.dependencies.compositor.install().await
        {
            journal.remove().await;
            return Err(SessionError::Refused(error.to_string()));
        }
        let (sender, receiver) = mpsc::channel(32);
        let (close, close_receiver) = mpsc::channel(1);
        let (events, event_receiver) = watch::channel(SessionSnapshot {
            control: crate::ipc::ControlState::Verifier,
            state: SessionState::Opening,
            checks: vec![],
            reminded: false,
            last_action: None,
        });
        // Set when a close is queued, before the actor can report it, and cleared if the actor refuses it.
        let closing = Arc::new(AtomicBool::new(false));
        let Some(arrival) = self.places.arrive(name.clone()) else {
            journal.remove().await;
            return Err(occupied());
        };
        if let Some(victim) = arrival.replaced
            && let Some(card) = self.cards.remove(&victim)
        {
            self.close_card(&victim, card.slot, card.frame).await;
        }
        self.workspace = Some(workspace);
        self.dependencies
            .viewer
            .opened(
                crate::viewer::SessionInfo {
                    name: name.clone(),
                    project: project.clone(),
                    branch: revision.branch.clone(),
                    goal: goal.clone(),
                    opened: started.with_timezone(&chrono::Utc),
                    slot,
                    placement: placement.geometry,
                    monitor: placement.monitor.clone(),
                    spot,
                },
                event_receiver.clone(),
            )
            .await;
        let (control, control_commands) = crate::session::control::Control::new();
        self.sessions.insert(
            name.clone(),
            Entry {
                card: CardDisposition::Keep,
                control: control.clone(),
                slot,
                closing: Arc::clone(&closing),
                worktree: worktree.clone(),
                mailbox: sender,
                close,
                events: event_receiver,
                window: window.clone(),
                window_size,
                still_warning: Arc::default(),
            },
        );
        self.arrange().await;
        Ok((
            Actor {
                test_files,
                frames_dir: self.dependencies.frames_dir.clone(),
                journal: Some(journal),
                idle_limit: self.dependencies.session_idle_limit,
                opener,
                control,
                control_commands: Some(control_commands),
                compositor: self.dependencies.compositor.clone(),
                opened_at,
                placement,
                window,
                credentials,
                project,
                worktree,
                profile,
                toolchains: Default::default(),
                warnings,
                goal,
                revision,
                started,
                checklist: crate::checklist::Checklist::new(checks),
                reminder: Default::default(),
                runner: Arc::clone(&self.dependencies.runner),
                closing,
                name,
                folder,
                factory: Arc::clone(&self.dependencies.factory),
                clock: Arc::clone(&self.dependencies.clock),
                catalog,
                events,
                progress,
                delegate: self.dependencies.delegate.clone(),
            },
            Inbox::new(receiver, close_receiver),
        ))
    }
}

fn occupied() -> SessionError {
    SessionError::Refused("No free Session Card slot. Close an Agent Session first.".into())
}

/// The worktree's canonical git root, refused when it isn't absolute, isn't a plain git worktree or sits inside the sessions folder.
pub(super) async fn checked_worktree(
    runner: &dyn ProcessRunner,
    sessions_dir: PathBuf,
    worktree: PathBuf,
) -> Result<(PathBuf, crate::revision::Revision), SessionError> {
    // The daemon's working folder is /, so a relative path would name the wrong folder.
    if !worktree.is_absolute() {
        return Err(SessionError::Refused(
            "worktree must be an absolute path. Pass the full path to a git worktree.".into(),
        ));
    }
    let worktree = tokio::task::spawn_blocking(move || std::fs::canonicalize(worktree))
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))?
        .map_err(|e| {
            SessionError::Refused(format!(
                "Cannot open worktree: {e}. Pass an existing git worktree."
            ))
        })?;
    let sessions = tokio::task::spawn_blocking(move || std::fs::canonicalize(sessions_dir))
        .await
        .map_err(|e| SessionError::Refused(e.to_string()))?;
    match &sessions {
        Ok(sessions) if worktree.starts_with(sessions) => {
            return Err(SessionError::Refused(
                "Cannot open a worktree inside the sessions folder. Choose a worktree outside it."
                    .into(),
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(SessionError::Refused(format!(
                "Cannot inspect sessions folder: {error}. Check its permissions."
            )));
        }
        _ => {}
    }
    let (root, revision) = revision(runner, &worktree).await?;
    let worktree = tokio::task::spawn_blocking(move || {
        let root = std::fs::canonicalize(root).map_err(|e| {
            format!("Cannot inspect git worktree root: {e}. Check the worktree and retry.")
        })?;
        crate::project::check_git_root(&worktree, &root).map_err(|e| e.to_string())?;
        Ok::<_, String>(root)
    })
    .await
    .map_err(|e| SessionError::Refused(e.to_string()))?
    .map_err(SessionError::Refused)?;
    if sessions
        .as_ref()
        .is_ok_and(|sessions| worktree.starts_with(sessions))
    {
        return Err(SessionError::Refused(
            "Cannot open a worktree inside the sessions folder. Choose a worktree outside it."
                .into(),
        ));
    }
    Ok((worktree, revision))
}
