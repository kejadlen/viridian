//! The editor loop: tempfile, `$EDITOR`, and the recovery file.

use std::collections::BTreeSet;
use std::process::Command;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use miette::Diagnostic;
use tempfile::NamedTempFile;
use thiserror::Error;
use viri_core::id::NodeId;
use viri_core::schema::ValidationError;

use crate::buffer;

/// Runs the editor on a node until it parses and validates, or the user
/// quits without fixing the errors.
///
/// Quitting with errors in the buffer keeps it in a recovery file under the
/// garden's `.viri/` directory and changes nothing.
pub fn run(
    garden_root: &Utf8Path,
    ids: &BTreeSet<NodeId>,
    start: buffer::Buffer,
    expected_id: &NodeId,
) -> Result<buffer::Buffer, EditorError> {
    let mut text = buffer::render(&start, &[]);
    loop {
        let file = NamedTempFile::with_suffix(".viri")?;
        fs_err::write(file.path(), &text)?;

        run_editor(file.path())?;
        let edited = fs_err::read_to_string(file.path())?;

        if edited == text {
            // The editor round produced exactly what we rendered. If that
            // had errors, the user quit without fixing them.
            match buffer::parse(&edited, expected_id) {
                Ok(parsed) => {
                    let errors = validate(&parsed, ids);
                    if errors.is_empty() {
                        return Ok(parsed);
                    }
                    return Err(save_recovery(
                        garden_root,
                        expected_id,
                        &edited,
                        errors.len(),
                    ));
                }
                Err(errors) => {
                    return Err(save_recovery(
                        garden_root,
                        expected_id,
                        &edited,
                        errors.len(),
                    ));
                }
            }
        }

        let errors = match buffer::parse(&edited, expected_id) {
            Ok(parsed) => {
                let errors = validate(&parsed, ids);
                if errors.is_empty() {
                    return Ok(parsed);
                }
                errors
            }
            Err(errors) => errors,
        };
        text = inject(&edited, &errors);
    }
}

/// The errors that apply to a parsed buffer, beyond what parsing checks.
fn validate(buffer: &buffer::Buffer, ids: &BTreeSet<NodeId>) -> Vec<ValidationError> {
    buffer.meta.validate(ids)
}

fn run_editor(path: &std::path::Path) -> Result<(), EditorError> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_owned());
    let mut words = editor.split_whitespace();
    let Some(program) = words.next() else {
        return Err(EditorError::NoEditor);
    };
    let status = Command::new(program)
        .args(words)
        .arg(path)
        .status()
        .map_err(|err| EditorError::Spawn {
            program: program.to_owned(),
            source: err,
        })?;
    if !status.success() {
        return Err(EditorError::Exited(status));
    }
    Ok(())
}

/// Inserts validation errors as comments at the top of the frontmatter.
fn inject(text: &str, errors: &[ValidationError]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# viri: {} error(s), fix them and save again\n",
        errors.len()
    ));
    for error in errors {
        out.push_str(&format!("# {error}\n"));
    }
    if let Some(rest) = text.strip_prefix("---\n") {
        out.push_str("---\n");
        out.push_str(rest);
    } else {
        out.push_str(text);
    }
    out
}

fn save_recovery(garden_root: &Utf8Path, id: &NodeId, text: &str, errors: usize) -> EditorError {
    let dir = garden_root.join(".viri").join("recovery");
    let recovery = dir.join(format!("{id}.viri"));
    let result = (|| -> std::io::Result<()> {
        fs_err::create_dir_all(&dir)?;
        fs_err::write(&recovery, text)?;
        let gitignore = garden_root.join(".gitignore");
        let existing = fs_err::read_to_string(&gitignore).unwrap_or_default();
        if !existing.lines().any(|line| line.trim() == ".viri/") {
            let mut updated = existing;
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(".viri/\n");
            fs_err::write(&gitignore, updated)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => EditorError::QuitWithErrors { recovery, errors },
        Err(source) => EditorError::Recovery {
            source,
            text: text.to_owned(),
        },
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum EditorError {
    #[error("no editor to run: $VISUAL and $EDITOR are empty")]
    #[diagnostic(code(viri::editor::no_editor))]
    NoEditor,

    #[error("failed to start the editor `{program}`")]
    #[diagnostic(
        code(viri::editor::spawn),
        help("set $VISUAL or $EDITOR to an installed editor")
    )]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the editor exited with {0}")]
    #[diagnostic(code(viri::editor::exited))]
    Exited(std::process::ExitStatus),

    #[error("quit with {errors} error(s); the buffer is in {recovery}")]
    #[diagnostic(
        code(viri::editor::quit_with_errors),
        help("edit the recovery file by hand or paste it back into `viri edit`")
    )]
    QuitWithErrors {
        recovery: Utf8PathBuf,
        errors: usize,
    },

    #[error("could not write the recovery file")]
    #[diagnostic(code(viri::editor::recovery))]
    Recovery {
        #[source]
        source: std::io::Error,
        text: String,
    },

    #[error(transparent)]
    #[diagnostic(code(viri::editor::io))]
    Io(#[from] std::io::Error),
}
