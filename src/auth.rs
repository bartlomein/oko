//! CLI credential management. Never print keys or credential-store error bodies.
use crate::config;
use anyhow::{Context, Result, bail};
use std::{env, io::IsTerminal, path::Path};
use zeroize::Zeroizing;

const SERVICE: &str = "io.github.bartlomein.oko";
const ACCOUNT: &str = "typesafe-api-key";
const STORE_ERROR: &str = "Cannot access the OS credential store. Unlock it and retry. On headless systems, set TYPESAFE_API_KEY in the environment or an ignored .env; Oko does not fall back to plaintext storage.";
const USAGE: &str = "Usage: oko auth login\n       oko auth status\n       oko auth logout\n\nLogin securely prompts for a TypeSafe AI key and replaces the saved key.\nPriority: environment > current directory .env > OS credential store.\nLogout removes the saved copy only; revoke the key through TypeSafe if needed.";

trait Store {
    fn get(&self) -> Result<Option<Zeroizing<String>>>;
    fn set(&self, value: &str) -> Result<()>;
    fn delete(&self) -> Result<bool>;
}
struct OsStore;
impl OsStore {
    fn entry() -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|_| anyhow::anyhow!(STORE_ERROR))
    }
}
impl Store for OsStore {
    fn get(&self) -> Result<Option<Zeroizing<String>>> {
        match Self::entry()?.get_password() {
            Ok(key) => Ok(Some(Zeroizing::new(key))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!(STORE_ERROR),
        }
    }
    fn set(&self, value: &str) -> Result<()> {
        Self::entry()?
            .set_password(value)
            .map_err(|_| anyhow::anyhow!(STORE_ERROR))
    }
    fn delete(&self) -> Result<bool> {
        match Self::entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => bail!(STORE_ERROR),
        }
    }
}

struct Credential {
    value: Zeroizing<String>,
    source: &'static str,
}
impl Credential {
    fn key(&self) -> Option<&str> {
        let value = config::trim(&self.value);
        (!value.is_empty()).then_some(value)
    }
}

// An explicitly empty override disables lower-priority credentials.
fn overrides(cwd: &Path) -> Result<Option<Credential>> {
    match env::var("TYPESAFE_API_KEY") {
        Ok(value) => {
            return Ok(Some(Credential {
                value: Zeroizing::new(value),
                source: "environment",
            }));
        }
        Err(env::VarError::NotUnicode(_)) => bail!("TYPESAFE_API_KEY must contain valid UTF-8."),
        Err(env::VarError::NotPresent) => {}
    }
    match std::fs::read(cwd.join(".env")) {
        Ok(bytes) => Ok(config::parse_env(&String::from_utf8_lossy(&bytes))
            .remove("TYPESAFE_API_KEY")
            .map(|value| Credential {
                value: Zeroizing::new(value),
                source: "current directory .env",
            })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("Cannot read the current directory .env."),
    }
}
fn resolve(override_key: Option<Credential>, store: &impl Store) -> Result<Option<Credential>> {
    if override_key.is_some() {
        return Ok(override_key);
    }
    Ok(store.get()?.map(|value| Credential {
        value,
        source: "OS credential store",
    }))
}
pub fn api_key(cwd: &Path) -> Result<Option<String>> {
    Ok(resolve(overrides(cwd)?, &OsStore)?.and_then(|c| c.key().map(str::to_owned)))
}

/// Prepare credentials that remain available when a GUI launches Oko.
pub(crate) fn setup(cwd: &Path) -> Result<()> {
    if let Some(credential) = overrides(cwd)? {
        let key = credential.key().context("An empty TYPESAFE_API_KEY override disables saved keys. Remove the empty override before setup.")?;
        if credential.source == "environment" {
            // GUI apps may not inherit the invoking terminal's environment.
            save(key, &OsStore)?;
            println!("TypeSafe key saved securely for use from Codex.");
        } else {
            println!("Using the project's .env key. Keep that file available and ignored by Git.");
        }
    } else if OsStore
        .get()?
        .is_some_and(|key| !config::trim(&key).is_empty())
    {
        println!("Using your saved TypeSafe key.");
    } else {
        run(&["login".into()], cwd)?;
    }
    Ok(())
}

fn status(override_key: Option<Credential>, store: &impl Store) -> Result<String> {
    Ok(match resolve(override_key, store)? {
        Some(c) if c.key().is_some() => format!(
            "TypeSafe AI key: configured\nActive source: {}\nKey validity has not been checked with TypeSafe.",
            c.source
        ),
        Some(c) => format!(
            "TypeSafe AI key: not configured\nActive source: {} (empty; lower-priority keys are not used).",
            c.source
        ),
        None => "TypeSafe AI key: not configured\nRun `oko auth login` to save a key.".into(),
    })
}
fn save(key: &str, store: &impl Store) -> Result<()> {
    let key = config::trim(key);
    if key.is_empty() {
        bail!("The key cannot be empty; the saved key was not changed.");
    }
    if key.len() > 4096 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!(
            "The key must be a single value without whitespace or control characters, at most 4096 bytes; the saved key was not changed."
        );
    }
    store.set(key)
}
fn override_notice(cwd: &Path) {
    match overrides(cwd) {
        Ok(Some(c)) => eprintln!(
            "Note: {} overrides the saved key, even when empty. This command does not change TYPESAFE_API_KEY in that source.",
            c.source
        ),
        Ok(None) => {}
        Err(_) => eprintln!(
            "Note: could not inspect credential overrides. Run `oko auth status` after checking your environment and .env."
        ),
    }
}
pub fn run(args: &[String], cwd: &Path) -> Result<()> {
    if args.is_empty() || matches!(args, [flag] if flag == "--help" || flag == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    // Do not echo unknown arguments: someone may accidentally pass their key.
    if args.len() != 1 || !matches!(args[0].as_str(), "login" | "status" | "logout") {
        bail!("Invalid auth command. Do not pass keys as arguments.\n\n{USAGE}");
    }
    match args[0].as_str() {
        "login" => {
            if !std::io::stdin().is_terminal() {
                bail!(
                    "Login requires an interactive terminal. For automation, set TYPESAFE_API_KEY in the environment."
                );
            }
            let key = Zeroizing::new(
                rpassword::prompt_password("TypeSafe AI API key (hidden): ")
                    .context("Could not read the hidden key; the saved key was not changed.")?,
            );
            save(&key, &OsStore)?;
            println!(
                "TypeSafe AI key saved in the OS credential store. Any previous saved key was replaced.\nThe key has not been checked with TypeSafe."
            );
            override_notice(cwd);
        }
        "status" => println!("{}", status(overrides(cwd)?, &OsStore)?),
        "logout" => {
            let deleted = OsStore.delete()?;
            println!(
                "{}\nThis does not revoke the key at TypeSafe or remove environment/.env keys.",
                if deleted {
                    "Saved TypeSafe AI key deleted."
                } else {
                    "No saved TypeSafe AI key found."
                }
            );
            override_notice(cwd);
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Default)]
    struct MemoryStore(RefCell<Option<String>>);
    impl Store for MemoryStore {
        fn get(&self) -> Result<Option<Zeroizing<String>>> {
            Ok(self.0.borrow().clone().map(Zeroizing::new))
        }
        fn set(&self, v: &str) -> Result<()> {
            *self.0.borrow_mut() = Some(v.into());
            Ok(())
        }
        fn delete(&self) -> Result<bool> {
            Ok(self.0.borrow_mut().take().is_some())
        }
    }
    struct Locked;
    impl Store for Locked {
        fn get(&self) -> Result<Option<Zeroizing<String>>> {
            bail!(STORE_ERROR)
        }
        fn set(&self, _: &str) -> Result<()> {
            bail!(STORE_ERROR)
        }
        fn delete(&self) -> Result<bool> {
            bail!(STORE_ERROR)
        }
    }
    // Opt-in only: ordinary tests never access the real credential store.
    #[test]
    #[ignore = "requires an unlocked OS credential store"]
    fn os_store_roundtrip() {
        let unique = format!(
            "test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let entry = keyring::Entry::new(&format!("{SERVICE}.test"), &unique).unwrap();
        struct Cleanup(keyring::Entry);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.delete_credential();
            }
        }
        let cleanup = Cleanup(entry);
        let entry = &cleanup.0;
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
        entry.set_password("oko-fake-test-key-one").unwrap();
        assert_eq!(entry.get_password().unwrap(), "oko-fake-test-key-one");
        entry.set_password("oko-fake-test-key-two").unwrap();
        assert_eq!(entry.get_password().unwrap(), "oko-fake-test-key-two");
        entry.delete_credential().unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }

    #[test]
    fn save_replace_delete_and_status_do_not_reveal_keys() {
        let store = MemoryStore::default();
        assert!(status(None, &store).unwrap().contains("not configured"));
        save(" first-secret ", &store).unwrap();
        assert_eq!(store.get().unwrap().unwrap().as_str(), "first-secret");
        save("second-secret", &store).unwrap();
        let text = status(None, &store).unwrap();
        assert!(text.contains("OS credential store"));
        assert!(!text.contains("second-secret"));
        assert!(store.delete().unwrap());
        assert!(!store.delete().unwrap());
        assert!(resolve(None, &store).unwrap().is_none());
    }
    #[test]
    fn explicit_overrides_including_empty_do_not_touch_locked_store() {
        for value in ["override-secret", "", "  "] {
            let c = Credential {
                value: Zeroizing::new(value.into()),
                source: "environment",
            };
            let result = resolve(Some(c), &Locked).unwrap().unwrap();
            assert_eq!(
                result.key(),
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value)
                }
            );
        }
        assert!(resolve(None, &Locked).is_err());
        assert!(save("valid-key", &Locked).is_err());
    }
    #[test]
    fn invalid_login_preserves_existing_key() {
        let store = MemoryStore::default();
        save("original", &store).unwrap();
        for value in [
            "",
            "   ",
            "two words",
            "secret\nsecond",
            "secret\0second",
            &"x".repeat(4097),
        ] {
            assert!(save(value, &store).is_err());
            assert_eq!(store.get().unwrap().unwrap().as_str(), "original");
        }
    }
}
