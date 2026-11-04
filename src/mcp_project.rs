use super::*;
use crate::ipc::CredentialSource;

pub(super) fn project_tools() -> Vec<Tool> {
    let credentials = json!({
        "type":"object",
        "description":"Test Credentials to copy into the Project's .env file. keek copies the values and returns only the names.",
        "properties":{
            "source":{"type":"string","description":"Absolute host path of a dotenv file to copy from, such as the repository's .env."},
            "names":{"type":"array","items":{"type":"string"},"description":"Names to copy. Names the source lacks are listed back."}
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
    [(
        "set_project_profile",
        "Validate and write the whole Project Profile, keeping the previous one as <project>.toml.bak. Returns the written TOML and the copied Test Credential names, never their values.",
        set,
    )]
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
    Ok(Request::SetProjectProfile {
        project: args.project,
        profile: args.profile,
        credentials: args.credentials,
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
        assert_eq!(properties, ["env", "install", "ports", "start", "window"]);
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
}
