//! Asking which garden a new node goes in, when nothing else says.

use std::io::BufRead;
use std::io::Write;

use camino::Utf8Path;
use miette::Diagnostic;
use thiserror::Error;

/// Lists `gardens` as numbered `(name, root)` choices and reads answers
/// until one is a listed number or a name only one garden has. Returns
/// the chosen index.
pub fn choose_garden(
    gardens: &[(&str, &Utf8Path)],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<usize, PromptError> {
    let width = gardens
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    writeln!(output, "Which garden should the new node go in?")?;
    for (number, (name, root)) in (1..).zip(gardens) {
        writeln!(output, "  {number}) {name:<width$}  {root}")?;
    }

    loop {
        write!(output, "Garden (number or name): ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Err(PromptError::NoAnswer);
        }
        let answer = line.trim();
        if let Some(index) = pick(gardens, answer) {
            return Ok(index);
        }
        let retry = format!(
            "`{answer}` isn't one of the gardens; answer 1-{} or a name",
            gardens.len()
        );
        writeln!(output, "{retry}")?;
    }
}

fn pick(gardens: &[(&str, &Utf8Path)], answer: &str) -> Option<usize> {
    if let Ok(number) = answer.parse::<usize>() {
        return number.checked_sub(1).filter(|index| *index < gardens.len());
    }
    let mut named = gardens
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| *name == answer);
    match (named.next(), named.next()) {
        (Some((index, _)), None) => Some(index),
        _ => None,
    }
}

#[derive(Error, Diagnostic, Debug)]
pub enum PromptError {
    #[error("failed to ask which garden")]
    #[diagnostic(code(viri::prompt::io))]
    Io(#[from] std::io::Error),

    #[error("no garden chosen")]
    #[diagnostic(
        code(viri::prompt::no_answer),
        help("pass --garden <name|path>, or set `default` in the viri config")
    )]
    NoAnswer,
}

#[cfg(test)]
mod tests {
    use super::*;

    const GARDENS: &[(&str, &str)] = &[
        ("ai", "/g/ai"),
        ("personal", "/g/personal"),
        ("twin", "/g/twin-a"),
        ("twin", "/g/twin-b"),
    ];

    fn ask(answers: &str) -> (Result<usize, PromptError>, String) {
        let gardens: Vec<(&str, &Utf8Path)> = GARDENS
            .iter()
            .map(|(name, root)| (*name, Utf8Path::new(root)))
            .collect();
        let mut output = Vec::new();
        let result = choose_garden(&gardens, &mut answers.as_bytes(), &mut output);
        (result, String::from_utf8(output).unwrap())
    }

    #[test]
    fn lists_the_gardens_aligned() {
        let (_, output) = ask("1\n");
        assert!(
            output.starts_with(concat!(
                "Which garden should the new node go in?\n",
                "  1) ai        /g/ai\n",
                "  2) personal  /g/personal\n",
                "  3) twin      /g/twin-a\n",
                "  4) twin      /g/twin-b\n",
                "Garden (number or name): ",
            )),
            "{output}"
        );
    }

    #[test]
    fn accepts_a_number_or_a_unique_name() {
        assert_eq!(ask("2\n").0.unwrap(), 1);
        assert_eq!(ask("  personal  \n").0.unwrap(), 1);
        assert_eq!(ask("4").0.unwrap(), 3);
    }

    #[test]
    fn asks_again_after_a_bad_answer() {
        for bad in ["0", "5", "work", "twin", ""] {
            let (result, output) = ask(&format!("{bad}\n1\n"));
            assert_eq!(result.unwrap(), 0, "{bad:?}");
            assert!(
                output.contains(&format!("`{bad}` isn't one of the gardens; answer 1-4")),
                "{output}"
            );
        }
    }

    #[test]
    fn end_of_input_is_no_answer() {
        let (result, _) = ask("work\n");
        assert!(matches!(result, Err(PromptError::NoAnswer)), "{result:?}");
    }

    #[test]
    fn a_failed_write_is_an_io_error() {
        // A full slice refuses further writes.
        let mut full: &mut [u8] = &mut [0; 4];
        let result = choose_garden(
            &[("ai", Utf8Path::new("/g/ai"))],
            &mut "1\n".as_bytes(),
            &mut full,
        );
        assert!(matches!(result, Err(PromptError::Io(_))), "{result:?}");
    }
}
