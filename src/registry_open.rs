use super::*;
impl Registry {
    pub(super) fn begin_open(
        &mut self,
        project: String,
        worktree: PathBuf,
        goal: String,
        checks: Vec<String>,
        reply: oneshot::Sender<Response>,
        mut opener: Option<watch::Receiver<()>>,
    ) {
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
                let (settings, mut warnings) = crate::project::read_project_settings(&project, &projects_config).await?;
                let credential_path = projects_config.join(format!("{project}.env"));
                let (credentials, warning) = tokio::task::spawn_blocking(move || crate::credentials::Credentials::read(&credential_path)).await.map_err(|e| SessionError::Refused(e.to_string()))?.map_err(|e| SessionError::Refused(e.to_string()))?;
                warnings.extend(warning);
                let goal = credentials.mask(&goal);
                let checks = checks.iter().map(|text| credentials.mask(text)).collect();
                let catalog = wait_catalog(&mut catalog)
                    .await
                    .map_err(SessionError::Refused)?;
                // The daemon's working folder is /, so a relative path would name the wrong folder.
                if !worktree.is_absolute() {
                    return Err(SessionError::Refused(
                        "worktree must be an absolute path. Pass the full path to a git worktree."
                            .into(),
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
                let sessions = tokio::task::spawn_blocking(move || std::fs::canonicalize(sessions_dir)).await.map_err(|e| SessionError::Refused(e.to_string()))?;
                match &sessions {
                    Ok(sessions) if worktree.starts_with(sessions) => return Err(SessionError::Refused("Cannot open a worktree inside the sessions folder. Choose a worktree outside it.".into())),
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(SessionError::Refused(format!("Cannot inspect sessions folder: {error}. Check its permissions."))),
                    _ => {}
                }
                let (root, revision) = revision(runner.as_ref(), &worktree).await?;
                let worktree = tokio::task::spawn_blocking(move || {
                    let root = std::fs::canonicalize(root).map_err(|e| format!("Cannot inspect git worktree root: {e}. Check the worktree and retry."))?;
                    crate::project::check_git_root(&worktree, &root).map_err(|e| e.to_string())?;
                    Ok::<_, String>(root)
                }).await.map_err(|e| SessionError::Refused(e.to_string()))?.map_err(SessionError::Refused)?;
                if sessions.as_ref().is_ok_and(|sessions| worktree.starts_with(sessions)) { return Err(SessionError::Refused("Cannot open a worktree inside the sessions folder. Choose a worktree outside it.".into())); }
                Ok(Opening {
                    credentials,
                    project,
                    worktree,
                    goal,
                    checks,
                    catalog,
                    revision,
                    settings, warnings,
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
            credentials,
            project,
            worktree,
            goal,
            checks,
            catalog,
            revision,
            settings,
            warnings,
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
        let free = (0..self.dependencies.cap).find(|index| {
            !self.sessions.values().any(|entry| entry.slot.0 == *index)
                && !self.cards.values().any(|card| card.slot.0 == *index)
        });
        let (slot, victim) = if let Some(index) = free {
            (crate::viewer::SlotId(index), None)
        } else if let Some((oldest, slot)) = self
            .cards
            .iter()
            .min_by_key(|(_, card)| (card.ended, card.slot.0))
            .map(|(name, card)| (name.clone(), card.slot))
        {
            (slot, Some(oldest))
        } else {
            return Err(SessionError::Refused(
                "No free Session Card slot. Close an Agent Session first.".into(),
            ));
        };
        let placement = self
            .dependencies
            .compositor
            .slot(slot, self.dependencies.cap, settings.window)
            .await
            .map_err(|e| SessionError::Refused(e.to_string()))?;
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
                credentials_hash: credentials.fingerprint(),
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
            last_action: None,
        });
        // Set when a close is queued, before the actor can report it, and cleared if the actor refuses it.
        let closing = Arc::new(AtomicBool::new(false));
        if let Some(victim) = victim {
            let frame = self.cards.remove(&victim).and_then(|card| card.frame);
            self.dependencies.viewer.closed(slot).await;
            remove_frame(frame).await;
        }
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
                    placement: placement.slot,
                    monitor: placement.monitor.clone(),
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
            },
        );
        Ok((
            Actor {
                frames_dir: self.dependencies.frames_dir.clone(),
                journal: Some(journal),
                idle_limit: self.dependencies.session_idle_limit,
                opener,
                control,
                control_commands: Some(control_commands),
                compositor: self.dependencies.compositor.clone(),
                opened_at,
                placement,
                credentials,
                project,
                worktree,
                settings,
                warnings,
                goal,
                revision,
                started,
                checklist: crate::checklist::Checklist::new(checks),
                runner: Arc::clone(&self.dependencies.runner),
                closing,
                name,
                folder,
                factory: Arc::clone(&self.dependencies.factory),
                clock: Arc::clone(&self.dependencies.clock),
                catalog,
                events,
            },
            Inbox::new(receiver, close_receiver),
        ))
    }
}
