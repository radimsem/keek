#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};
    use keek::{
        checklist::{Checklist, SessionEnd, Verdict},
        evidence,
        report::{IdentityTimeMap, ReportInputs},
        revision::Revision,
    };
    use std::{fs, path::Path};

    fn tree(path: &Path) -> Vec<String> {
        fn walk(root: &Path, path: &Path, out: &mut Vec<String>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
                if path.is_dir() {
                    walk(root, &path, out);
                }
            }
        }
        let mut out = vec![];
        walk(path, path, &mut out);
        out.sort();
        out
    }
    fn fixture(root: &Path) -> std::path::PathBuf {
        let folder = root.join("session");
        fs::create_dir(&folder).unwrap();
        let source = Path::new("tests/fixtures/sessions/myapp_14-03-22");
        for name in tree(source) {
            let dest = folder.join(&name);
            if source.join(&name).is_dir() {
                fs::create_dir(dest).unwrap();
            } else {
                fs::copy(source.join(name), dest).unwrap();
            }
        }
        folder
    }
    fn file(folder: &Path, videos: &Path, verdict: Verdict) -> evidence::Filed {
        let revision = Revision {
            branch: Some("fix/17-parser-bom".into()),
            commit: "a1b2c3d456".into(),
            dirty: true,
        };
        let end = SessionEnd::Verdict(verdict);
        evidence::file(
            folder,
            videos,
            "myapp",
            "myapp_14-03-22",
            Local.with_ymd_and_hms(2026, 10, 30, 14, 3, 22).unwrap(),
            &ReportInputs {
                session: "myapp_14-03-22",
                goal: "parser works",
                revision: &revision,
                summary: "checked",
                checks: Checklist::new(vec![]).checks(),
                end: &end,
                time_map: &IdentityTimeMap,
                video_error: None,
            },
        )
        .unwrap()
    }
    #[test]
    fn filing_preserves_the_exact_layout_and_optional_video() {
        for video in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let folder = fixture(root.path());
            if video {
                fs::write(folder.join("verification.mp4"), b"edited").unwrap();
            }
            let videos = root.path().join("videos");
            let filed = file(&folder, &videos, Verdict::NeedsFixes);
            assert!(!folder.exists());
            assert_eq!(filed.verification.is_some(), video);
            assert_eq!(
                serde_json::to_value(&filed).unwrap()["verification"].is_null(),
                !video
            );
            let mut expected = vec!["index.md".to_string(), "2026-10-30".into()];
            for suffix in [
                "actions",
                "actions/turn-00001",
                "actions/turn-00001/action.json",
                "actions/turn-00002",
                "actions/turn-00002/action.json",
                "raw.mp4",
                "report.md",
                "timeline.json",
            ] {
                expected.push(format!("2026-10-30/myapp_14-03-22_needs-fixes_{suffix}"));
            }
            if video {
                expected.push("2026-10-30/myapp_14-03-22_needs-fixes_verification.mp4".into());
            }
            expected.sort();
            assert_eq!(tree(&videos.join("keek/myapp")), expected);
        }
    }
    #[test]
    fn collisions_preserve_every_byte_of_prior_evidence() {
        for verdict in [Verdict::Pass, Verdict::NeedsFixes] {
            let root = tempfile::tempdir().unwrap();
            let videos = root.path().join("videos");
            file(&fixture(root.path()), &videos, Verdict::Pass);
            let before: Vec<_> = tree(&videos)
                .into_iter()
                .filter(|p| videos.join(p).is_file())
                .filter(|p| !p.ends_with("index.md"))
                .map(|p| {
                    let bytes = fs::read(videos.join(&p)).unwrap();
                    (p, bytes)
                })
                .collect();
            let second = file(&fixture(root.path()), &videos, verdict);
            assert!(
                second
                    .report
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("myapp_14-03-22-2_")
            );
            for (p, bytes) in before {
                assert_eq!(fs::read(videos.join(p)).unwrap(), bytes);
            }
        }
    }
    #[test]
    fn an_existing_suffix_counts_up_from_the_session_name() {
        let root = tempfile::tempdir().unwrap();
        let videos = root.path().join("videos");
        let revision = Revision {
            branch: None,
            commit: "abcdefghi".into(),
            dirty: false,
        };
        let end = SessionEnd::Verdict(Verdict::Pass);
        let input = ReportInputs {
            session: "myapp_14-03-22-2",
            goal: "g",
            revision: &revision,
            summary: "",
            checks: &[],
            end: &end,
            time_map: &IdentityTimeMap,
            video_error: None,
        };
        for expected in [
            "myapp_14-03-22-2_pass_report.md",
            "myapp_14-03-22-3_pass_report.md",
        ] {
            let filed = evidence::file(
                &fixture(root.path()),
                &videos,
                "myapp",
                "myapp_14-03-22-2",
                Local.with_ymd_and_hms(2026, 10, 30, 14, 3, 22).unwrap(),
                &input,
            )
            .unwrap();
            assert_eq!(filed.report.file_name().unwrap(), expected);
        }
    }

    #[test]
    fn filing_skips_symlinks_and_fifos_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let folder = fixture(root.path());
        let target = root.path().join("private");
        fs::write(&target, b"private contents").unwrap();
        fs::remove_file(folder.join("recording.mp4")).unwrap();
        symlink(&target, folder.join("recording.mp4")).unwrap();
        symlink(&target, folder.join("turn-00001/private")).unwrap();
        symlink(root.path(), folder.join("turn-00009")).unwrap();
        let fifo = std::ffi::CString::new(
            folder
                .join("turn-00001/fifo")
                .as_os_str()
                .as_encoded_bytes(),
        )
        .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let videos = root.path().join("videos");
        let filed = file(&folder, &videos, Verdict::Pass);
        assert!(filed.raw.is_none());
        assert_eq!(fs::read(target).unwrap(), b"private contents");
        assert!(!tree(&videos).iter().any(|p| p.contains("turn-00009")));
        for p in tree(&videos) {
            if videos.join(&p).is_file() {
                assert!(
                    !fs::read(videos.join(p))
                        .unwrap()
                        .windows(16)
                        .any(|w| w == b"private contents")
                );
            }
        }
    }
    #[test]
    fn concurrent_index_appends_are_whole_lines() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("index.md");
        let threads: Vec<_> = (0..20)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    evidence::append_index(&path, &format!("line {i}\n")).unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = fs::read_to_string(path).unwrap();
        let mut lines: Vec<_> = text.lines().collect();
        lines.sort();
        lines.dedup();
        assert_eq!(lines.len(), 20);
        for i in 0..20 {
            assert!(lines.contains(&format!("line {i}").as_str()));
        }
    }

    #[tokio::test]
    async fn videos_dir_uses_videos_and_refuses_empty_or_relative_output() {
        struct Runner(&'static str);
        impl keek::process::ProcessRunner for Runner {
            fn output<'a>(
                &'a self,
                program: &'a str,
                args: &'a [&'a str],
                cwd: Option<&'a Path>,
            ) -> keek::process::ProcessFuture<'a> {
                Box::pin(async move {
                    assert_eq!(program, "xdg-user-dir");
                    assert_eq!(args, ["VIDEOS"]);
                    assert!(cwd.is_none());
                    Ok(self.0.into())
                })
            }
        }
        for value in ["", "   ", "relative"] {
            assert!(
                evidence::videos_dir(&Runner(value))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("absolute VIDEOS folder")
            );
        }
        assert_eq!(
            evidence::videos_dir(&Runner("/tmp/videos\n"))
                .await
                .unwrap(),
            Path::new("/tmp/videos")
        );
    }
    #[test]
    fn a_symlinked_timeline_keeps_the_session_folder() {
        let root = tempfile::tempdir().unwrap();
        let folder = fixture(root.path());
        fs::remove_file(folder.join("timeline.json")).unwrap();
        std::os::unix::fs::symlink(folder.join("recording.mp4"), folder.join("timeline.json"))
            .unwrap();
        let revision = Revision {
            branch: None,
            commit: "abcdefghi".into(),
            dirty: false,
        };
        let error = evidence::file(
            &folder,
            &root.path().join("videos"),
            "myapp",
            "myapp_14-03-22",
            Local::now(),
            &ReportInputs {
                session: "myapp_14-03-22",
                goal: "g",
                revision: &revision,
                summary: "",
                checks: &[],
                end: &SessionEnd::Verdict(Verdict::Pass),
                time_map: &IdentityTimeMap,
                video_error: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("timeline.json"));
        assert!(folder.join("recording.mp4").exists());
    }
    #[test]
    fn a_missing_timeline_keeps_the_session_folder() {
        let root = tempfile::tempdir().unwrap();
        let folder = fixture(root.path());
        fs::remove_file(folder.join("timeline.json")).unwrap();
        let revision = Revision {
            branch: None,
            commit: "abcdefghi".into(),
            dirty: false,
        };
        let error = evidence::file(
            &folder,
            &root.path().join("videos"),
            "myapp",
            "myapp_14-03-22",
            Local::now(),
            &ReportInputs {
                session: "myapp_14-03-22",
                goal: "g",
                revision: &revision,
                summary: "",
                checks: &[],
                end: &SessionEnd::Verdict(Verdict::Pass),
                time_map: &IdentityTimeMap,
                video_error: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains(folder.to_str().unwrap()));
        assert!(folder.join("recording.mp4").exists());
    }
}
