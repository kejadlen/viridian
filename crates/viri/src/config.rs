//! The user config: the gardens `viri` knows by name, and the default.
//!
//! The file is `$XDG_CONFIG_HOME/viri/config.kdl`, or
//! `~/.config/viri/config.kdl` when that's unset, on every platform:
//!
//! ```kdl
//! garden "~/notes/personal"
//! garden "/srv/gardens/public"
//! default "personal"
//! ```
//!
//! Each `garden` line is a garden's root. Its name comes from the
//! `garden "<name>"` line in that root's `viri.kdl`, so it's written in
//! one place only.

use camino::Utf8Path;
use camino::Utf8PathBuf;
use kdl::KdlDocument;
use kdl::KdlError;
use kdl::KdlNode;
use miette::Diagnostic;
use thiserror::Error;
use viri_core::meta::Meta;
use viri_core::meta::MetaError;

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    /// Garden roots, with `~` expanded.
    pub gardens: Vec<Utf8PathBuf>,
    /// The name of the garden to use outside any garden.
    pub default: Option<String>,
}

impl Config {
    /// Loads the config from its XDG location. A missing file is an empty
    /// config, so `viri` works without one.
    pub fn load() -> Result<Config, ConfigError> {
        let home = env_var("HOME")?.map(Utf8PathBuf::from);
        let xdg = env_var("XDG_CONFIG_HOME")?;
        let path = config_path(xdg.as_deref(), home.as_deref());
        Config::load_from(path.as_deref(), home.as_deref())
    }

    pub fn load_from(
        path: Option<&Utf8Path>,
        home: Option<&Utf8Path>,
    ) -> Result<Config, ConfigError> {
        let Some(path) = path else {
            return Ok(Config::default());
        };
        match fs_err::read_to_string(path) {
            Ok(text) => Config::parse(path, &text, home),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(err) => Err(ConfigError::Io {
                path: path.to_owned(),
                source: err,
            }),
        }
    }

    pub fn parse(
        path: &Utf8Path,
        text: &str,
        home: Option<&Utf8Path>,
    ) -> Result<Config, ConfigError> {
        let doc: KdlDocument = text.parse().map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        let invalid = |message: String| ConfigError::Invalid {
            path: path.to_owned(),
            message,
        };

        let mut config = Config::default();
        for node in doc.nodes() {
            match node.name().value() {
                "garden" => {
                    let raw = only_string_arg(node).map_err(invalid)?;
                    let root = expand(&raw, home).map_err(invalid)?;
                    if config.gardens.contains(&root) {
                        return Err(invalid(format!("garden `{raw}` is listed twice")));
                    }
                    config.gardens.push(root);
                }
                "default" => {
                    if config.default.is_some() {
                        return Err(invalid("`default` appears twice".to_owned()));
                    }
                    config.default = Some(only_string_arg(node).map_err(invalid)?);
                }
                other => {
                    return Err(invalid(format!(
                        "unexpected node `{other}`, expected `garden` or `default`"
                    )));
                }
            }
        }
        Ok(config)
    }

    /// The root of the one configured garden whose `viri.kdl` is named
    /// `name`. Unreadable gardens are skipped, so one broken garden doesn't
    /// hide the rest, unless none of the others match.
    pub fn find(&self, name: &str) -> Result<Utf8PathBuf, ConfigError> {
        let mut matches = Vec::new();
        let mut others = Vec::new();
        let mut unreadable = None;
        for root in &self.gardens {
            match Meta::open(&root.join("viri.kdl")) {
                Ok(meta) if meta.garden_name() == name => matches.push(root.clone()),
                Ok(meta) => others.push(meta.garden_name().to_owned()),
                Err(err) => {
                    unreadable.get_or_insert(err);
                }
            }
        }
        match (matches.as_slice(), unreadable) {
            ([root], _) => Ok(root.clone()),
            ([], Some(source)) => Err(ConfigError::Unreadable {
                name: name.to_owned(),
                source,
            }),
            ([], None) => Err(ConfigError::UnknownGarden {
                name: name.to_owned(),
                help: if others.is_empty() {
                    "no gardens are configured; list them as `garden \"<path>\"` lines in the viri config".to_owned()
                } else {
                    format!("the configured gardens are named {}", others.join(", "))
                },
            }),
            _ => Err(ConfigError::AmbiguousGarden {
                name: name.to_owned(),
                roots: matches
                    .iter()
                    .map(|root| root.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }
}

/// Where the config lives. A relative `XDG_CONFIG_HOME` is ignored, as the
/// XDG spec requires.
pub fn config_path(xdg_config_home: Option<&str>, home: Option<&Utf8Path>) -> Option<Utf8PathBuf> {
    let base = xdg_config_home
        .map(Utf8PathBuf::from)
        .filter(|base| base.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))?;
    Some(base.join("viri").join("config.kdl"))
}

/// An environment variable, with empty treated as unset.
fn env_var(name: &'static str) -> Result<Option<String>, ConfigError> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::NonUtf8Env { name }),
    }
}

fn expand(raw: &str, home: Option<&Utf8Path>) -> Result<Utf8PathBuf, String> {
    let home = || home.ok_or_else(|| format!("garden `{raw}` needs $HOME, which is unset"));
    if raw == "~" {
        return home().map(Utf8Path::to_owned);
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home().map(|home| home.join(rest));
    }
    let path = Utf8PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "garden `{raw}` is relative; use an absolute path or one starting with `~/`"
        ))
    }
}

fn only_string_arg(node: &KdlNode) -> Result<String, String> {
    let name = node.name().value();
    let mut args = node.entries().iter().filter(|e| e.name().is_none());
    match (args.next(), args.next()) {
        (Some(arg), None) => arg
            .value()
            .as_string()
            .map(str::to_owned)
            .ok_or_else(|| format!("`{name}` expects a quoted string argument")),
        _ => Err(format!("`{name}` expects exactly one argument")),
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum ConfigError {
    #[error("${name} is not valid UTF-8")]
    #[diagnostic(code(viri::config::non_utf8_env))]
    NonUtf8Env { name: &'static str },

    #[error("failed to read {}", .path)]
    #[diagnostic(code(viri::config::io))]
    Io {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse {}", .path)]
    #[diagnostic(code(viri::config::parse))]
    Parse {
        path: Utf8PathBuf,
        #[source]
        source: KdlError,
    },

    #[error("{}: {message}", .path)]
    #[diagnostic(code(viri::config::invalid))]
    Invalid { path: Utf8PathBuf, message: String },

    #[error("no garden is named `{name}`")]
    #[diagnostic(code(viri::config::unknown_garden))]
    UnknownGarden {
        name: String,
        #[help]
        help: String,
    },

    #[error("more than one garden is named `{name}`: {roots}")]
    #[diagnostic(
        code(viri::config::ambiguous_garden),
        help("rename one in its viri.kdl, or pass --garden <path>")
    )]
    AmbiguousGarden { name: String, roots: String },

    #[error("no readable garden is named `{name}`")]
    #[diagnostic(
        code(viri::config::unreadable_garden),
        help("fix the garden below, or remove it from the viri config")
    )]
    Unreadable {
        name: String,
        #[source]
        source: MetaError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/config/viri/config.kdl";

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(PATH.into(), text, Some("/home/me".into()))
    }

    fn invalid(text: &str) -> String {
        let err = parse(text).expect_err("rejected");
        assert!(matches!(err, ConfigError::Invalid { .. }), "{err:?}");
        err.to_string()
    }

    fn help(err: &ConfigError) -> String {
        miette::Diagnostic::help(err)
            .map(|help| help.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn parses_gardens_and_a_default() {
        let config = parse(
            "garden \"~/notes\"\ngarden \"/srv/public\"\ngarden \"~\"\ndefault \"personal\"\n",
        )
        .unwrap();
        assert_eq!(
            config,
            Config {
                gardens: vec![
                    "/home/me/notes".into(),
                    "/srv/public".into(),
                    "/home/me".into()
                ],
                default: Some("personal".to_owned()),
            }
        );
    }

    #[test]
    fn an_empty_file_is_an_empty_config() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn rejects_invalid_configs() {
        for (text, expected) in [
            ("garden \"notes\"\n", "is relative"),
            ("garden \"/a\"\ngarden \"/a\"\n", "listed twice"),
            ("garden \"~/a\"\ngarden \"/home/me/a\"\n", "listed twice"),
            ("default \"a\"\ndefault \"b\"\n", "appears twice"),
            ("gardens \"/a\"\n", "unexpected node `gardens`"),
            ("garden\n", "exactly one argument"),
            ("garden \"/a\" \"/b\"\n", "exactly one argument"),
            ("default 42\n", "quoted string"),
        ] {
            let message = invalid(text);
            assert!(message.contains(expected), "{text:?}: {message}");
        }
    }

    #[test]
    fn tilde_needs_home() {
        for text in ["garden \"~/notes\"\n", "garden \"~\"\n"] {
            let err = Config::parse(PATH.into(), text, None).expect_err("rejected");
            assert!(err.to_string().contains("needs $HOME"), "{err}");
        }
    }

    #[test]
    fn rejects_malformed_kdl() {
        assert!(matches!(
            parse("garden \"unterminated"),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn xdg_config_home_wins_over_home() {
        assert_eq!(
            config_path(Some("/xdg"), Some("/home/me".into())),
            Some("/xdg/viri/config.kdl".into())
        );
    }

    #[test]
    fn a_relative_xdg_config_home_falls_back_to_home() {
        assert_eq!(
            config_path(Some("xdg"), Some("/home/me".into())),
            Some("/home/me/.config/viri/config.kdl".into())
        );
        assert_eq!(
            config_path(None, Some("/home/me".into())),
            Some("/home/me/.config/viri/config.kdl".into())
        );
    }

    #[test]
    fn no_xdg_config_home_or_home_means_no_config() {
        assert_eq!(config_path(Some("xdg"), None), None);
        assert_eq!(Config::load_from(None, None).unwrap(), Config::default());
    }

    #[test]
    fn a_missing_file_is_an_empty_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(dir.path().join("config.kdl")).unwrap();
        assert_eq!(
            Config::load_from(Some(&path), None).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn an_unreadable_file_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(dir.path().to_owned()).unwrap();
        assert!(matches!(
            Config::load_from(Some(&path), None),
            Err(ConfigError::Io { .. })
        ));
    }

    fn garden(dir: &tempfile::TempDir, sub: &str, viri_kdl: &str) -> Utf8PathBuf {
        let root = Utf8PathBuf::try_from(dir.path().join(sub)).unwrap();
        fs_err::create_dir(&root).unwrap();
        fs_err::write(root.join("viri.kdl"), viri_kdl).unwrap();
        root
    }

    #[test]
    fn find_matches_the_name_in_viri_kdl() {
        let dir = tempfile::TempDir::new().unwrap();
        let personal = garden(&dir, "p", "garden \"personal\"\n");
        let public = garden(&dir, "q", "garden \"public\"\n");
        let broken = garden(&dir, "b", "not a garden\n");
        let config = Config {
            gardens: vec![broken, personal.clone(), public.clone()],
            default: None,
        };
        assert_eq!(config.find("personal").unwrap(), personal);
        assert_eq!(config.find("public").unwrap(), public);
    }

    #[test]
    fn find_lists_the_known_names_for_an_unknown_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            gardens: vec![
                garden(&dir, "p", "garden \"personal\"\n"),
                garden(&dir, "q", "garden \"public\"\n"),
            ],
            default: None,
        };
        let err = config.find("work").expect_err("unknown");
        assert!(matches!(err, ConfigError::UnknownGarden { .. }), "{err:?}");
        assert!(help(&err).contains("personal, public"), "{}", help(&err));
        let err = Config::default().find("work").expect_err("unknown");
        assert!(
            help(&err).contains("no gardens are configured"),
            "{}",
            help(&err)
        );
    }

    #[test]
    fn find_reports_an_unreadable_garden_when_nothing_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            gardens: vec![
                garden(&dir, "p", "garden \"personal\"\n"),
                Utf8PathBuf::try_from(dir.path().join("missing")).unwrap(),
            ],
            default: None,
        };
        assert!(matches!(
            config.find("work"),
            Err(ConfigError::Unreadable { .. })
        ));
    }

    #[test]
    fn find_rejects_a_name_two_gardens_share() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            gardens: vec![
                garden(&dir, "a", "garden \"personal\"\n"),
                garden(&dir, "b", "garden \"personal\"\n"),
            ],
            default: None,
        };
        let err = config.find("personal").expect_err("ambiguous");
        assert!(
            matches!(err, ConfigError::AmbiguousGarden { .. }),
            "{err:?}"
        );
    }
}
