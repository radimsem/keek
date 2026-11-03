use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Attempt(pub usize);
pub fn count(index: &str, branch: Option<&str>) -> Option<Attempt> {
    let branch = branch?;
    Some(Attempt(
        index
            .lines()
            .filter(|line| {
                let fields: Vec<_> = line.split(" | ").collect();
                fields.len() >= 6
                    && fields[1] == branch
                    && chrono::NaiveDateTime::parse_from_str(fields[0], "%Y-%m-%d %H:%M:%S").is_ok()
                    && fields[2].len() == 7
                    && fields[2].bytes().all(|b| b.is_ascii_hexdigit())
                    && matches!(
                        fields[3],
                        "Pass" | "Needs fixes" | "Partly verified" | "Incomplete"
                    )
            })
            .count(),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_only_valid_lines_for_this_branch() {
        let fixture = "2026-11-03 12:00:00 | feat/x | abc1234 | Pass | goal | [Report](x)\n2026-11-03 12:01:00 | feat/y | abc1234 | Pass | goal | [Report](y)\n2026-11-03 12:02:00 | feat/x | def5678 | Needs fixes | goal | [Report](z)\n2026-11-03 12:03:00 | detached HEAD | abc1234 | Pass | goal | [Report](z)\nbroken | feat/x\n";
        assert_eq!(count(fixture, Some("feat/x")), Some(Attempt(2)));
        assert_eq!(count(fixture, None), None);
    }
}
