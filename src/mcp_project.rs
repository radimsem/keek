use super::*;
use crate::ipc::CredentialSource;

pub(super) fn project_tools() -> Vec<Tool> {
    let credentials = json!({
        "type":"object",
        "description":"Test Credentials to copy into the Project's .env file. keek copies the values and returns only the names.",
        "properties":{
            "source":{"type":"string","description":"Absolute host path of a dotenv file to copy from, such as the repository's .env."},
            "names":{"type":"array","items":{"type":"string"},"description":"Names to copy. Names the source lacks are listed back."},
            "public":{"type":"array","items":{"type":"string"},"description":"Names from names whose values may appear in Evidence without masking. Other copied names stay secret."}
        },
        "required":["source","names"]
    });
    let set = json!({
        "type":"object",
        "properties":{
            "project":{"type":"string"},
            "profile":profile_schema(),
            "credentials":credentials
        },
        "required":["project","profile"]
    });
    let prepare = json!({
        "type":"object",
        "properties":{"project":{"type":"string"},"worktree":{"type":"string"}},
        "required":["project","worktree"]
    });
    [
        (
            "set_project_profile",
            "Validate and write the whole Project Profile, keeping the previous one as <project>.toml.bak. Returns the written TOML and the copied Test Credential names, never their values.",
            set,
        ),
        (
            "prepare_project",
            "Build or reuse the Project Layer, then run start for up to 15 seconds with no desktop or recording. Returns the toolchain versions, whether the layer was built or reused, the install and start log tails, and whether start was still running or exited with a status. If this call is cancelled the build keeps running, and calling prepare_project again joins it.",
            prepare,
        ),
    ]
    .into_iter()
    .map(|(name, description, schema)| {
        Tool::new(name, description, schema.as_object().cloned().unwrap_or_default())
    })
    .collect()
}

pub(super) fn profile_schema() -> Value {
    let mut generator = schemars::generate::SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator();
    let mut schema = generator
        .root_schema_for::<crate::project::Profile>()
        .to_value();
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
    }
    schema
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArguments {
    project: String,
    profile: Value,
    credentials: Option<CredentialSource>,
}

pub(super) fn parse_set_profile(value: Value) -> Result<Request, String> {
    let args: SetArguments = serde_json::from_value(value).map_err(|e| {
        format!("Cannot set the Project Profile: {e}. Pass project and profile, and credentials with source and names to copy Test Credentials.")
    })?;
    if let Some(credentials) = &args.credentials {
        for name in &credentials.public {
            if !credentials.names.contains(name) {
                return Err(crate::project::CopyError::UnrequestedPublic(name.clone()).to_string());
            }
        }
    }
    Ok(Request::SetProjectProfile {
        project: args.project,
        profile: args.profile,
        credentials: args.credentials,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareArguments {
    project: String,
    worktree: std::path::PathBuf,
}

pub(super) fn parse_prepare(value: Value) -> Result<Request, String> {
    let args: PrepareArguments = serde_json::from_value(value)
        .map_err(|e| format!("Cannot prepare the Project: {e}. Pass project and worktree."))?;
    if !args.worktree.is_absolute() {
        return Err("Cannot prepare the Project: worktree must be an absolute path. Pass the full path to a git worktree.".into());
    }
    Ok(Request::PrepareProject {
        project: args.project,
        worktree: args.worktree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_schema_lists_every_profile_key_and_requires_start() {
        let schema = profile_schema();
        let properties: Vec<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            properties,
            ["env", "install", "ports", "start", "toolchain", "window"]
        );
        assert_eq!(
            schema["properties"]["toolchain"]["additionalProperties"],
            json!(false)
        );
        assert_eq!(schema["required"], json!(["start"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert!(!schema.to_string().contains("$ref"), "{schema}");
    }

    #[test]
    fn set_project_profile_passes_the_profile_through_for_the_daemon_to_check() {
        let request = parse_set_profile(json!({"project":"web","profile":{"start":"npm start"},"credentials":{"source":"/repo/.env","names":["TEST_PASSWORD"]}})).unwrap();
        let Request::SetProjectProfile {
            project,
            profile,
            credentials,
        } = request
        else {
            panic!("{request:?}");
        };
        assert_eq!(project, "web");
        assert_eq!(profile, json!({"start":"npm start"}));
        assert_eq!(credentials.unwrap().names, ["TEST_PASSWORD"]);
        assert!(parse_set_profile(json!({"project":"web"})).is_err());
    }

    #[test]
    fn set_project_profile_refuses_a_public_name_outside_the_names_to_copy() {
        let error = parse_set_profile(json!({
            "project":"web", "profile":{"start":"npm start"},
            "credentials":{"source":"/repo/.env", "names":["PASSWORD"], "public":["USER"]}
        }))
        .unwrap_err();
        assert!(
            error.contains("USER") && error.contains("names") && error.contains("public"),
            "{error}"
        );
    }

    #[test]
    fn prepare_project_refuses_a_relative_worktree_and_may_wait() {
        let error = parse_prepare(json!({"project":"web","worktree":"relative"})).unwrap_err();
        assert!(error.contains("absolute path"), "{error}");
        let request = parse_prepare(json!({"project":"web","worktree":"/repo"})).unwrap();
        assert!(super::super::can_wait(&request));
    }
}

#[cfg(test)]
mod skill_tests {
    use super::*;

    const SKILL: &str = include_str!("../skills/keek-setup/SKILL.md");

    fn keys(schema: &Value) -> Vec<String> {
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn the_setup_skill_example_is_a_valid_profile_using_every_schema_key() {
        let example = SKILL
            .split("```toml\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("the skill has a toml example");
        let profile =
            crate::project::Profile::parse(std::path::Path::new("SKILL.md"), example).unwrap();
        let written: Value = toml::from_str(example).unwrap();
        let schema = profile_schema();
        let mut used: Vec<String> = written.as_object().unwrap().keys().cloned().collect();
        used.sort();
        assert_eq!(used, keys(&schema));
        for tool in keys(&schema["properties"]["toolchain"]) {
            assert!(
                SKILL.contains(&format!("`{tool}`")),
                "the skill does not name toolchain {tool}"
            );
        }
        assert!(!profile.toolchain.is_empty());
    }
}
