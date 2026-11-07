//! The same steps on this computer: the installer runs here, its output streamed the way a
//! server's is. On Linux `sudo -S -v` takes the password once up front, then the installer's own
//! `sudo` calls use that; on a Mac the node runs in the person's account and needs none; on
//! Windows the PowerShell installer runs for this account.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use crate::ssh::{SetupError, Stream};

/// Runs `program` with `args`, writes `stdin` to it, and hands every line over as it comes.
pub fn local_exec(program: &str, args: &[String], stdin: Option<&str>, mut on_line: impl FnMut(Stream, String) + Send + 'static) -> Result<i32, SetupError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SetupError::Unreachable(format!("{program}: {e}")))?;
    if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
        let _ = input.write_all(text.as_bytes());
        let _ = input.write_all(b"\n");
    }
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = std::sync::mpsc::channel::<(Stream, String)>();
    let out_tx = tx.clone();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = out_tx.send((Stream::Out, line));
        }
    });
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send((Stream::Err, line));
        }
    });
    for (stream, line) in rx {
        on_line(stream, line);
    }
    let _ = out_thread.join();
    let _ = err_thread.join();
    let status = child.wait()?;
    Ok(status.code().unwrap_or(255))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    #[cfg(unix)]
    fn output_arrives_line_by_line_from_both_streams() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let code = local_exec("sh", &["-c".into(), "echo one; echo two >&2; cat; exit 3".into()], Some("from stdin"), move |s, l| sink.lock().unwrap().push((s, l))).unwrap();
        assert_eq!(code, 3);
        let lines = got.lock().unwrap().clone();
        assert!(lines.contains(&(Stream::Out, "one".to_string())));
        assert!(lines.contains(&(Stream::Err, "two".to_string())));
        assert!(lines.contains(&(Stream::Out, "from stdin".to_string())));
    }
}
