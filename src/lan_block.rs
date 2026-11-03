use crate::sandbox::DNS_FORWARDER;

#[derive(Debug, thiserror::Error)]
pub enum LanBlockError {
    #[error("Can't start ip: {source}. Install iproute2, then open again.")]
    Spawn { source: std::io::Error },
    #[error(
        "Can't block private networks for the Agent Session: `{command}` failed: {detail}. Check that the kernel allows ip rules, then open again."
    )]
    Rule { command: String, detail: String },
}

pub trait IpTool {
    fn run(&self, args: &[&str]) -> Result<(), LanBlockError>;
}

pub struct HostIp;

impl IpTool for HostIp {
    fn run(&self, args: &[&str]) -> Result<(), LanBlockError> {
        run_ip("ip", args)
    }
}

fn run_ip(program: &str, args: &[&str]) -> Result<(), LanBlockError> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|source| LanBlockError::Spawn { source })?;
    if !output.status.success() {
        return Err(LanBlockError::Rule {
            command: format!("ip {}", args.join(" ")),
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

const IPV4_RANGES: [&str; 5] = [
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "100.64.0.0/10",
    "169.254.0.0/16",
];
const IPV6_RANGES: [&str; 2] = ["fc00::/7", "fe80::/10"];

pub fn block_lan(tool: &dyn IpTool) -> Result<(), LanBlockError> {
    tool.run(&[
        "-4",
        "rule",
        "add",
        "to",
        &format!("{DNS_FORWARDER}/32"),
        "lookup",
        "main",
        "pref",
        "10",
    ])?;
    for (family, ranges) in [
        ("-4", IPV4_RANGES.as_slice()),
        ("-6", IPV6_RANGES.as_slice()),
    ] {
        for range in ranges {
            tool.run(&[
                family,
                "rule",
                "add",
                "to",
                range,
                "unreachable",
                "pref",
                "20",
            ])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn a_missing_ip_binary_says_to_install_iproute2() {
        let error = run_ip("/nonexistent-keek-ip", &["-4", "rule"]).unwrap_err();
        assert!(matches!(error, LanBlockError::Spawn { .. }));
        assert!(error.to_string().contains("ip"));
        assert!(error.to_string().contains("Install iproute2"));
    }

    #[test]
    fn a_nonzero_ip_exit_names_the_command_and_trims_stderr() {
        let args = ["-c", "printf ' denied \n' >&2; exit 7"];
        let error = run_ip("sh", &args).unwrap_err();
        match error {
            LanBlockError::Rule { command, detail } => {
                assert_eq!(command, format!("ip {}", args.join(" ")));
                assert_eq!(detail, "denied");
            }
            LanBlockError::Spawn { source } => panic!("unexpected spawn failure: {source}"),
        }
    }

    #[derive(Default)]
    struct FakeIp {
        calls: RefCell<Vec<Vec<String>>>,
        fail_at: Option<usize>,
    }
    impl IpTool for FakeIp {
        fn run(&self, args: &[&str]) -> Result<(), LanBlockError> {
            let mut calls = self.calls.borrow_mut();
            calls.push(args.iter().map(|s| (*s).to_owned()).collect());
            if self.fail_at == Some(calls.len() - 1) {
                return Err(LanBlockError::Rule {
                    command: format!("ip {}", args.join(" ")),
                    detail: "Operation not permitted".into(),
                });
            }
            Ok(())
        }
    }

    #[test]
    fn blocking_makes_every_private_range_unreachable_in_both_families() {
        let tool = FakeIp::default();
        block_lan(&tool).unwrap();
        let expected = [
            "-4 rule add to 169.254.1.1/32 lookup main pref 10",
            "-4 rule add to 10.0.0.0/8 unreachable pref 20",
            "-4 rule add to 172.16.0.0/12 unreachable pref 20",
            "-4 rule add to 192.168.0.0/16 unreachable pref 20",
            "-4 rule add to 100.64.0.0/10 unreachable pref 20",
            "-4 rule add to 169.254.0.0/16 unreachable pref 20",
            "-6 rule add to fc00::/7 unreachable pref 20",
            "-6 rule add to fe80::/10 unreachable pref 20",
        ];
        let actual: Vec<_> = tool.calls.borrow().iter().map(|a| a.join(" ")).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn the_dns_forwarder_keeps_a_route_ahead_of_the_blocked_ranges() {
        let tool = FakeIp::default();
        block_lan(&tool).unwrap();
        let calls = tool.calls.borrow();
        assert_eq!(calls.len(), 8);
        assert_eq!(
            calls[0],
            [
                "-4",
                "rule",
                "add",
                "to",
                &format!("{DNS_FORWARDER}/32"),
                "lookup",
                "main",
                "pref",
                "10"
            ]
        );
        for block in &calls[1..] {
            assert_eq!(block.last().unwrap(), "20");
        }
    }

    #[test]
    fn a_failing_rule_stops_the_blocking_and_names_the_command() {
        for index in 0..8 {
            let tool = FakeIp {
                fail_at: Some(index),
                ..Default::default()
            };
            let error = block_lan(&tool).unwrap_err().to_string();
            let calls = tool.calls.borrow();
            assert_eq!(calls.len(), index + 1);
            assert!(error.contains(&format!("ip {}", calls[index].join(" "))));
            assert!(error.contains("Operation not permitted"));
        }
    }
}
