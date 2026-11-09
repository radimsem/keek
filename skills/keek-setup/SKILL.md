---
name: keek-setup
description: Write a keek Project Profile and prove it starts the app. Use before a Project's first open_session, when open_session refuses for a missing or invalid Project Profile, or when its install or start fails.
---

# keek setup

A Project Profile tells keek how to install and start a Project's App Under Test in a sandbox on this machine. You write it with `set_project_profile` and try it with `prepare_project`. keek detects nothing itself: every value comes from what you read in the repository.

## Steps

1. **Read the repository.** Find what installs it and what starts it: the lockfile (`package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `uv.lock`, `poetry.lock`), the `package.json` scripts or `Makefile` targets, version files (`.nvmrc`, `.node-version`, `.tool-versions`, `rust-toolchain.toml`, `.python-version`, the `engines` and `packageManager` fields), the README's run instructions, and `.env.example` or `docker-compose.yml` for the variables and services the app expects. Done when you can name the install command, the start command, each pinned toolchain version, every variable the app reads, and every host service it connects to.
2. **Write the profile** with `set_project_profile(project, profile, credentials?)`, choosing values with the rules below. Done when it returns the written TOML. A refusal names the key and the reason; fix that key and call again.
3. **Prove it** with `prepare_project(project, worktree)`. Done when the reply has `start.state` `running` and `start_log` shows the app ready, such as a listening line. Otherwise read `install_log`, `start_log` and the refusal, change the profile in step 2, and call again. A cancelled `prepare_project` keeps building; call it again to join the same build.
4. **Report** to the developer the Test Credential names `set_project_profile` listed as `missing`, and the host services the app needs running, such as a database in Docker.

## Profile format

The profile is one TOML table. Only `start` is required. `set_project_profile` takes the same keys as JSON.

```toml
install = "pnpm install --frozen-lockfile"
start = "pnpm dev --host 127.0.0.1 --port 5173"
ports = [5432]

[toolchain]
node = "24"
pnpm = "9"

[env]
APP_ENV = "test"
DATABASE_URL = "postgres://app@127.0.0.1:5432/app_test"

[window]
width = 1280
height = 800
```

## Rules

- **install** runs once per Project Layer with `bash -c` from the worktree root. Only its writes inside the worktree are kept, such as `node_modules`, `.venv` or `target`; HOME and `/tmp` are thrown away. Use the lockfile's frozen install: `npm ci`, `pnpm install --frozen-lockfile`, `yarn install --immutable`, `uv sync --frozen`, `cargo build --locked`. Leave it out when nothing needs installing. It has 20 minutes and has internet access.
- **start** runs in every Agent Session from the worktree root and keeps running. Bind the app to `127.0.0.1`, because the Session's browser runs in the same sandbox. A command that builds and then serves belongs in `start`; only the dependency install belongs in `install`. A script it calls is an ordinary repository file.
- **Browser**: when `start` opens the App Under Test in Chromium, disable its password manager before launching it. Every Agent Session gets a fresh HOME, so after each sign-in Chromium otherwise offers to save the password in a bubble with the username over the page. An app window takes the Session Window's exact size, so a phone size gives a phone-width page. Use this recipe, replacing `<url>` with the app's URL:

  ```sh
  mkdir -p "$HOME/chromium/Default" && printf '%s' '{"credentials_enable_service":false,"profile":{"password_manager_enabled":false}}' > "$HOME/chromium/Default/Preferences" && exec chromium --enable-logging=stderr --ozone-platform=wayland --kiosk --app=<url> --no-first-run --no-default-browser-check --password-store=basic --user-data-dir="$HOME/chromium"
  ```

- **toolchain** pins `node`, `rust`, `python`, `pnpm` and `yarn`, using the version the repository's own files name, such as `"24"` from an `.nvmrc` of `24` or `"1.90.0"` from `rust-toolchain.toml`. keek installs each pin once into a read-only store and puts it on `PATH`. Without a pin the sandbox uses `/usr/bin`, whose version may differ from what the repository expects.
- **env** holds non-secret values for both install and start, such as `APP_ENV` or a local service URL. The Verifier can read them, and a change rebuilds the Project Layer. keek refuses the names it sets itself, such as `HOME`, `PATH`, `DISPLAY` and those starting `KEEK_`, `XDG_` or `WAYLAND_`, and the refusal names the one to remove.
- **Test Credentials** hold test-only usernames, passwords, tokens and keys. Copy them with `credentials: { "source": "<absolute path to the repository's .env>", "names": ["TEST_USER", "TEST_PASSWORD"] }`. keek copies the values into its own file and returns only the names; you never see a value, and the Verifier types `${TEST_USER}` or `${TEST_PASSWORD}`. Copied values are masked in results. If a username is an ordinary word that masking would blank out of the report, ask the developer to add the line `# keek: public TEST_USER` to `~/.config/keek/projects/<project>.env`; only the developer can make a credential public. Passwords, tokens and keys stay secret. A credential and an `[env]` entry with the same name: the credential wins.
- **ports** lists host loopback ports to forward into the sandbox, so the app reaches a host service such as Postgres on `127.0.0.1:5432`. An Agent Session blocks the LAN, so a service on another machine needs a host loopback tunnel and its port here.
- **window** is the Session Window size, 1280x800 without it. Use a phone size such as 390x844 for a mobile layout, with the Browser recipe above.
