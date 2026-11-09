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
    fingerprint: Option<String>,
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
pub(crate) fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub(crate) fn public_name(line: &str) -> Option<&str> {
    line.strip_prefix("# keek: public ")
        .filter(|name| valid_name(name))
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
/// One `NAME=value` line of an env file, with its 1-based line number.
pub(crate) struct Entry<'a> {
    pub line: usize,
    pub name: &'a str,
    pub value: &'a str,
}
/// Reads every `NAME=value` line, refusing malformed ones, without the length and duplicate rules.
pub(crate) fn entries<'a>(path: &Path, text: &'a str) -> Result<Vec<Entry<'a>>, CredentialsError> {
    let mut found = Vec::new();
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
        let value = value.strip_suffix('\r').unwrap_or(value);
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value.trim()
        };
        found.push(Entry {
            line: index + 1,
            name,
            value,
        });
    }
    Ok(found)
}
pub(crate) fn check_length(path: &Path, name: &str, value: &str) -> Result<(), CredentialsError> {
    if value.chars().count() < 6 {
        return Err(CredentialsError::Short {
            path: path.into(),
            name: name.into(),
        });
    }
    Ok(())
}
// One variable rather than one per name, so a credential named PATH can't clash with the environment pasta inherits.
pub(crate) const SANDBOX_ENV: &str = "KEEK_CREDENTIALS";

impl Credentials {
    pub fn parse(path: &Path, text: &str) -> Result<Self, CredentialsError> {
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        let public: Vec<&str> = text.lines().filter_map(public_name).collect();
        for entry in entries(path, text)? {
            if pairs.contains_key(entry.name) {
                return Err(CredentialsError::Duplicate {
                    path: path.into(),
                    line: entry.line,
                    name: entry.name.into(),
                });
            }
            // A public value is never masked, so a short one can't corrupt other text.
            if !public.contains(&entry.name) {
                check_length(path, entry.name, entry.value)?;
            }
            pairs.insert(entry.name.into(), entry.value.into());
        }
        let mut forms = Vec::new();
        for (name, value) in &pairs {
            if public.contains(&name.as_str()) {
                continue;
            }
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
        Ok(Self {
            pairs,
            forms,
            fingerprint: Some({
                use sha2::{Digest, Sha256};
                Sha256::digest(text.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }),
        })
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
    pub fn fingerprint(&self) -> Option<String> {
        self.fingerprint.clone()
    }
    /// The byte length of the longest form `mask` replaces.
    pub fn longest_form(&self) -> usize {
        self.forms.iter().map(String::len).max().unwrap_or(0)
    }
    pub fn names(&self) -> Vec<String> {
        self.pairs.keys().cloned().collect()
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
    pub(crate) fn sandbox_env(&self) -> Option<String> {
        (!self.pairs.is_empty()).then(|| {
            self.pairs
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
    }
    // Names are rechecked here because a value is only ever as trusted as the environment it arrived in.
    pub(crate) fn from_sandbox_env(text: &str) -> impl Iterator<Item = (&str, &str)> {
        text.split('\n').filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            valid_name(name).then_some((name, value))
        })
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
    fn a_public_username_stays_readable_while_a_secret_password_is_masked() {
        let credentials = Credentials::parse(
            Path::new("fixture.env"),
            "# keek: public USER\n# keek: public ABSENT\nUSER=operator\nPASSWORD=made-up-password\n",
        )
        .unwrap();
        assert_eq!(
            credentials.mask("operator made-up-password"),
            "operator [masked]"
        );
        assert_eq!(
            credentials.mask_value(json!({"operator": "operator made-up-password"})),
            json!({"operator": "operator [masked]"})
        );
        assert_eq!(credentials.names(), ["PASSWORD", "USER"]);
        let mut arguments = json!("${USER} ${PASSWORD}");
        credentials.substitute(&mut arguments).unwrap();
        assert_eq!(arguments, "operator made-up-password");
    }

    #[test]
    fn public_values_have_no_masking_forms() {
        let credentials = Credentials::parse(
            Path::new("fixture.env"),
            "USER=public\"user\\name\n# keek: public USER\n",
        )
        .unwrap();
        assert_eq!(credentials.longest_form(), 0);
        let text = r#"public"user\name public\"user\\name public%22user%5Cname"#;
        assert_eq!(credentials.mask(text), text);
    }

    #[test]
    fn a_short_public_value_is_accepted_while_a_short_secret_is_refused() {
        let credentials = Credentials::parse(
            Path::new("fixture.env"),
            "# keek: public USER\nUSER=admin\nPASSWORD=made-up-password\n",
        )
        .unwrap();
        assert_eq!(credentials.mask("admin made-up-password"), "admin [masked]");
        let error = Credentials::parse(Path::new("fixture.env"), "USER=admin\n").unwrap_err();
        assert!(matches!(error, CredentialsError::Short { .. }), "{error}");
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
    #[test]
    fn the_sandbox_environment_holds_one_pair_per_line_in_name_order() {
        assert_eq!(
            credentials().sandbox_env().as_deref(),
            Some("OTHER=long enough\nPASSWORD=pa\"ss\\w&rd 1")
        );
        assert_eq!(Credentials::default().sandbox_env(), None);
    }
}
