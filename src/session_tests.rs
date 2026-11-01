use super::*;
#[test]
fn timeline_times_saturate_and_subtract_the_video_lead() {
    let zero = Instant::now();
    assert_eq!(
        timeline_time(zero, zero + Duration::from_millis(50)),
        Millis(0)
    );
    assert_eq!(
        timeline_time(zero, zero + Duration::from_millis(216)),
        Millis(100)
    );
}
#[test]
fn missing_host_environment_is_named() {
    let host = HostEnvironment {
        wayland: None,
        signature: Some("fake".into()),
    };
    assert!(
        host.validate()
            .unwrap_err()
            .to_string()
            .contains("WAYLAND_DISPLAY")
    );
}
#[test]
fn choose_name_increments_collisions_and_refuses_invalid_projects() {
    let used = std::collections::HashSet::from(["app_12-30-04".to_owned()]);
    assert_eq!(
        choose_name("app", "12-30-04", |name| used.contains(name))
            .unwrap()
            .to_string(),
        "app_12-30-04-2"
    );
    assert!(choose_name("../app", "12-30-04", |_| false).is_err());
}

#[tokio::test]
async fn git_revision_disables_fsmonitor_and_hooks_for_every_call() {
    struct Runner(std::sync::Mutex<Vec<Vec<String>>>);
    impl crate::process::ProcessRunner for Runner {
        fn output<'a>(
            &'a self,
            _: &'a str,
            args: &'a [&'a str],
            _: Option<&'a Path>,
        ) -> crate::process::ProcessFuture<'a> {
            Box::pin(async move {
                self.0
                    .lock()
                    .unwrap()
                    .push(args.iter().map(|s| (*s).to_owned()).collect());
                Ok("/tmp/repo".into())
            })
        }
    }
    let runner = Runner(std::sync::Mutex::new(vec![]));
    revision(&runner, Path::new("/tmp/repo")).await.unwrap();
    for call in runner.0.lock().unwrap().iter() {
        assert_eq!(
            &call[..5],
            [
                "--no-lazy-fetch",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null"
            ]
        );
    }
}
