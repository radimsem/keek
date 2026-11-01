use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CredentialsError {
    #[error("Cannot read Test Credentials {path}: {source}. Check the file and its permissions.")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "Malformed Test Credentials in {path} at line {line}. Use one NAME=value per line with a valid name."
    )]
    Malformed { path: PathBuf, line: usize },
    #[error("Duplicate Test Credential {name} in {path} at line {line}. Keep each name once.")]
    Duplicate {
        path: PathBuf,
        line: usize,
        name: String,
    },
    #[error(
        "Test Credential {name} in {path} has fewer than 6 characters. A short value would corrupt unrelated text when masked. Use a longer one."
    )]
    Short { path: PathBuf, name: String },
    #[error(
        "Unknown Test Credential {name}. Known names: {known}. Use a known name. In run_script, which is JavaScript, build text with + rather than a template literal."
    )]
    Unknown { name: String, known: String },
}
#[derive(Clone, Default)]
pub struct Credentials {
    pairs: BTreeMap<String, String>,
    forms: Vec<String>,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("names", &self.names())
            .finish()
    }
}
fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn json_form(text: &str) -> String {
    let encoded = Value::String(text.into()).to_string();
    encoded[1..encoded.len() - 1].into()
}
fn percent_form(text: &str, raw: &[u8], lower: bool, plus: bool) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || raw.contains(&b) {
            out.push(b as char);
        } else if plus && b == b' ' {
            out.push('+');
        } else if lower {
            out.push_str(&format!("%{b:02x}"));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
impl Credentials {
    pub fn parse(path: &Path, text: &str) -> Result<Self, CredentialsError> {
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let malformed = || CredentialsError::Malformed {
                path: path.into(),
                line: index + 1,
            };
            let (name, value) = line.split_once('=').ok_or_else(malformed)?;
            if !valid_name(name) {
                return Err(malformed());
            }
            if pairs.contains_key(name) {
                return Err(CredentialsError::Duplicate {
                    path: path.into(),
                    line: index + 1,
                    name: name.into(),
                });
            }
            let value = value.strip_suffix('\r').unwrap_or(value);
            let value = if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                &value[1..value.len() - 1]
            } else {
                value.trim()
            };
            if value.chars().count() < 6 {
                return Err(CredentialsError::Short {
                    path: path.into(),
                    name: name.into(),
                });
            }
            pairs.insert(name.into(), value.into());
        }
        let mut forms = Vec::new();
        for value in pairs.values() {
            forms.extend([
                value.clone(),
                json_form(value),
                json_form(&json_form(value)),
            ]);
            // encodeURIComponent leaves !*'() raw, and Python's quote leaves / raw.
            for raw in [&b""[..], b"!*'()", b"/"] {
                for lower in [false, true] {
                    for plus in [false, true] {
                        forms.push(percent_form(value, raw, lower, plus));
                    }
                }
            }
        }
        forms.sort();
        forms.dedup();
        forms.sort_by_key(|s| std::cmp::Reverse(s.len()));
        Ok(Self { pairs, forms })
    }
    pub fn read(path: &Path) -> Result<(Self, Option<String>), CredentialsError> {
        let read_error = |source| CredentialsError::Read {
            path: path.into(),
            source,
        };
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Self::default(), None));
            }
            Err(e) => return Err(read_error(e)),
        };
        let mode = file.metadata().map_err(read_error)?.permissions().mode() & 0o777;
        let mut text = String::new();
        file.read_to_string(&mut text).map_err(read_error)?;
        let credentials = Self::parse(path, &text)?;
        let warning = (mode & 0o044 != 0).then(|| format!("Test Credentials {} has mode {mode:03o}, allowing other users to read it. Run chmod 600 {}.", path.display(), path.display()));
        Ok((credentials, warning))
    }
    pub fn names(&self) -> Vec<String> {
        self.pairs.keys().cloned().collect()
    }
    pub(crate) fn pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
    pub fn substitute(&self, arguments: &mut Value) -> Result<(), CredentialsError> {
        match arguments {
            Value::String(text) => {
                let mut out = String::new();
                let mut rest = text.as_str();
                while let Some(start) = rest.find("${") {
                    out.push_str(&rest[..start]);
                    rest = &rest[start..];
                    if let Some(end) = rest.find('}') {
                        let name = &rest[2..end];
                        if valid_name(name) {
                            let value =
                                self.pairs
                                    .get(name)
                                    .ok_or_else(|| CredentialsError::Unknown {
                                        name: name.into(),
                                        known: if self.pairs.is_empty() {
                                            "none".into()
                                        } else {
                                            self.names().join(", ")
                                        },
                                    })?;
                            out.push_str(value);
                            rest = &rest[end + 1..];
                            continue;
                        }
                    }
                    out.push_str("${");
                    rest = &rest[2..];
                }
                out.push_str(rest);
                *text = out;
            }
            Value::Array(items) => {
                for item in items {
                    self.substitute(item)?;
                }
            }
            Value::Object(items) => {
                for item in items.values_mut() {
                    self.substitute(item)?;
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
        Ok(())
    }
    pub fn mask(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(form) = self
                .forms
                .iter()
                .find(|form| rest.starts_with(form.as_str()))
            {
                out.push_str("[masked]");
                rest = &rest[form.len()..];
            } else if let Some(c) = rest.chars().next() {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        out
    }
    pub fn mask_value(&self, value: Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.mask(&s)),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|v| self.mask_value(v)).collect())
            }
            Value::Object(items) => {
                let media = matches!(
                    items.get("type").and_then(Value::as_str),
                    Some("image" | "audio")
                );
                Value::Object(
                    items
                        .into_iter()
                        .map(|(k, v)| {
                            let v = if media && k == "data" {
                                v
                            } else {
                                self.mask_value(v)
                            };
                            (self.mask(&k), v)
                        })
                        .collect(),
                )
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => value,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn credentials() -> Credentials {
        Credentials::parse(
            Path::new("fixture.env"),
            "PASSWORD=pa\"ss\\w&rd 1\nOTHER=long enough",
        )
        .unwrap()
    }
    #[test]
    fn parsing_preserves_literal_values_and_accepts_quotes_export_and_comments() {
        let c=Credentials::parse(Path::new("fixture.env"), " # comment\n\nexport Z='quoted value'\r\nA=  unquoted value  \nB=\" quoted value \"\nPASSWORD=pa\"ss\\w&rd 1").unwrap();
        assert_eq!(c.names(), ["A", "B", "PASSWORD", "Z"]);
        let mut v = json!(["${PASSWORD}", "${A}", "${B}", "${Z}"]);
        c.substitute(&mut v).unwrap();
        assert!(
            v == json!([
                "pa\"ss\\w&rd 1",
                "unquoted value",
                " quoted value ",
                "quoted value"
            ])
        );
    }
    #[test]
    fn malformed_lines_report_only_the_path_and_line() {
        let e = Credentials::parse(Path::new("fixture.env"), "\nprivate malformed text")
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("fixture.env") && e.contains('2') && !e.contains("private malformed text")
        );
    }
    #[test]
    fn short_values_are_refused_without_echoing_them() {
        let e = Credentials::parse(Path::new("fixture.env"), "PASSWORD=p4ss")
            .unwrap_err()
            .to_string();
        assert!(e.contains("PASSWORD") && !e.contains("p4ss"));
        assert!(Credentials::parse(Path::new("fixture.env"), "PASSWORD=ééééé").is_err());
    }
    #[test]
    fn duplicate_names_are_refused() {
        assert!(Credentials::parse(Path::new("fixture.env"), "A=abcdef\nA=ghijkl").is_err());
    }
    #[test]
    fn substitution_walks_nested_arguments_and_run_steps() {
        let mut v = json!({"steps":[{"arguments":{"text":"${PASSWORD}"}}]});
        credentials().substitute(&mut v).unwrap();
        assert!(v["steps"][0]["arguments"]["text"] == "pa\"ss\\w&rd 1");
    }
    #[test]
    fn substituted_values_are_not_rescanned() {
        let c = Credentials::parse(Path::new("fixture.env"), "A=${OTHER}").unwrap();
        let mut v = json!("${A}");
        c.substitute(&mut v).unwrap();
        assert_eq!(v, json!("${OTHER}"));
    }
    #[test]
    fn unknown_names_list_known_names() {
        let e = credentials()
            .substitute(&mut json!("${MISSING}"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("OTHER") && e.contains("PASSWORD") && e.contains("run_script"));
    }
    #[test]
    fn invalid_placeholders_stay_literal() {
        let mut v = json!("${a b} ${unterminated");
        credentials().substitute(&mut v).unwrap();
        assert_eq!(v, json!("${a b} ${unterminated"));
    }
    #[test]
    fn debug_shows_names_without_values() {
        let s = format!("{:?}", credentials());
        assert!(s.contains("PASSWORD") && !s.contains("pa\"ss"));
    }
    #[test]
    fn masking_removes_raw_json_and_url_forms() {
        let c = credentials();
        let raw = "pa\"ss\\w&rd 1";
        let once = serde_json::to_string(raw).unwrap();
        let escaped = &once[1..once.len() - 1];
        let twice = serde_json::to_string(escaped).unwrap();
        for form in [
            raw,
            escaped,
            &twice[1..twice.len() - 1],
            "pa%22ss%5Cw%26rd%201",
            "pa%22ss%5cw%26rd+1",
        ] {
            assert_eq!(c.mask(form), "[masked]");
        }
    }
    #[test]
    fn masking_covers_url_encoders_that_leave_punctuation_raw() {
        let c = Credentials::parse(Path::new("fixture.env"), "A=Pass word!1/x").unwrap();
        for form in ["Pass%20word!1%2Fx", "Pass%20word%211/x", "Pass+word!1%2fx"] {
            assert_eq!(c.mask(form), "[masked]");
        }
    }
    #[test]
    fn longer_values_are_masked_first() {
        let c = Credentials::parse(Path::new("fixture.env"), "A=abcdef\nB=abcdefghi").unwrap();
        assert_eq!(c.mask("abcdefghi"), "[masked]");
    }
    #[test]
    fn masking_preserves_media_data_and_masks_keys() {
        let v=credentials().mask_value(json!({"pa\"ss\\w&rd 1":{"type":"image","data":"pa\"ss\\w&rd 1","text":"pa\"ss\\w&rd 1"}}));
        assert!(v["[masked]"]["data"] == "pa\"ss\\w&rd 1");
        assert_eq!(v["[masked]"]["text"], "[masked]");
    }

    #[test]
    fn reading_credentials_warns_for_readable_modes_and_accepts_missing_files() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("fixture.env");
        let (empty, warning) = Credentials::read(&path).unwrap();
        assert!(empty.names().is_empty());
        assert!(warning.is_none());
        std::fs::write(&path, "PASSWORD=made-up-value").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let (c, warning) = Credentials::read(&path).unwrap();
        assert_eq!(c.names(), ["PASSWORD"]);
        let warning = warning.unwrap();
        assert!(
            warning.contains("644")
                && warning.contains("chmod 600")
                && warning.contains(path.to_str().unwrap())
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Credentials::read(&path).unwrap().1.is_none());
        assert!(Credentials::read(root.path()).is_err());
    }
}
