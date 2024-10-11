use std::{
    env,
    fmt::Display,
    fs,
    io::{self, stdout, Write},
    net::{Ipv4Addr, TcpListener},
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
        unix::{net::UnixDatagram, prelude::MetadataExt},
    },
    path::PathBuf,
    process::{exit, Command},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// A dumb implementation of /dev/log that dumps messages to stdout.
fn dumb_syslog() {
    let listener = UnixDatagram::bind("/dev/log").unwrap();
    let mut stdout = stdout();
    eprintln!("approx syslog listening on /dev/log");

    let mut buffer = [0u8; 8192];
    loop {
        if let Ok(size) = listener.recv(&mut buffer) {
            stdout.write(&buffer[..size]).unwrap();
        }
    }
}

/// A dumb implementation of inetd that launches approx from one port
fn dumb_inetd(port: u16, target: Vec<String>) -> ! {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).unwrap();
    eprintln!("approx inetd listening on port {0}", port);

    loop {
        let (peer, _) = listener.accept().unwrap();
        let (peer_stdin, peer_stdout, peer_stderr) = dup_socket_for_stdio(peer.as_fd()).unwrap();

        let mut child = Command::new(&target[0])
            .args(&target[1..])
            .stdin(peer_stdin)
            .stdout(peer_stdout)
            .stderr(peer_stderr)
            .spawn()
            .unwrap();

        thread::spawn(move || {
            child.wait().unwrap()
        });
    }
}

/// An approx garbage collector that deletes old cache files.
fn garbage_collect_cache(interval: Duration, max_age: Duration, cache_root: &str) {
    let mut last_run = Instant::now();
    loop {
        let since_last_run = Instant::now().duration_since(last_run);
        match interval.checked_sub(since_last_run) {
            None => (),
            Some(idle) => thread::sleep(idle),
        }

        eprintln!("approx gc performing scheduled run");
        last_run = Instant::now();
        let current_time_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let target_time_unix = current_time_unix - max_age.as_secs();

        let mut dir_counter = 0;
        let mut file_counter = 0;
        let mut del_counter = 0;
        let mut to_visit = vec![PathBuf::from(cache_root)];
        while let Some(visiting) = to_visit.pop() {
            for entry in fs::read_dir(visiting).unwrap().map(|e| e.unwrap()) {
                let ft = entry.file_type().unwrap();
                if ft.is_dir() {
                    dir_counter += 1;
                    to_visit.push(entry.path());
                } else if ft.is_file() {
                    file_counter += 1;

                    // Use the ctime rather than the mtime. The mtime is set to
                    // the time when the file in the repository was updated,
                    // while the ctime is set when approx touches the file:
                    //
                    // https://salsa.debian.org/ocaml-team/approx/-/blob/9e06b4e0ce4fb6a3e7d92efdb014f538412f407b/approx.ml#L76
                    let last_modified_unix = entry.metadata().unwrap().ctime() as u64;
                    if last_modified_unix < target_time_unix {
                        fs::remove_file(entry.path()).unwrap();
                        del_counter += 1;
                    }
                }
            }
        }

        eprintln!(
            "approx gc completed dirs={0} files={1} deleted={2}",
            dir_counter, file_counter, del_counter
        );
    }
}

/// Duplicates file descriptors for a bidirectional file three times. Unlike
/// `try_clone_to_owned`, this does not use the close-on-exec flag and is
/// suitable for passing to subprocesses.
fn dup_socket_for_stdio(fd: BorrowedFd<'_>) -> io::Result<(OwnedFd, OwnedFd, OwnedFd)> {
    unsafe {
        let raw_fd = fd.as_raw_fd();
        let a = libc::fcntl(raw_fd, libc::F_DUPFD, 3);
        if a == -1 {
            return Err(io::Error::last_os_error());
        }

        let b = libc::fcntl(raw_fd, libc::F_DUPFD, 3);
        if b == -1 {
            return Err(io::Error::last_os_error());
        }

        let c = libc::fcntl(raw_fd, libc::F_DUPFD, 3);
        if c == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok((
            OwnedFd::from_raw_fd(a),
            OwnedFd::from_raw_fd(b),
            OwnedFd::from_raw_fd(c),
        ))
    }
}

/// Parses Duration values from strings. Duration values are a positive number
/// of seconds in the form `[[[DD:]HH:]MM:]SS`.
fn parse_duration(value: &str) -> Result<Duration, &'static str> {
    let mut segments: [u64; 4] = [0; 4];
    let scales: [u64; 4] = [86400, 3600, 60, 1];
    let mut cursor = 0;

    for ch in value.chars() {
        match ch {
            '0'..='9' => segments[cursor] = (segments[cursor] * 10) + (ch as u64 - '0' as u64),
            ':' => {
                if cursor == 3 {
                    return Err("Duration has too many colons, can only be DD:HH:MM:SS");
                }
                cursor = cursor + 1;
            }
            _ => return Err("Duration can only contain colons and digits"),
        }
    }

    // For example: 01:02:03 fills segments as [1, 2, 3, _] but requires scales [3600, 60, 0]
    let used_segs = segments.iter().take(cursor + 1);
    let used_scales = scales.iter().skip(3 - cursor);
    let total = used_segs
        .zip(used_scales)
        .map(|(seg, scale)| seg * scale)
        .sum();
    Ok(Duration::from_secs(total))
}

fn usage(message: impl Display) -> ! {
    eprintln!(
        "{0}
Usage: approx-host (syslog ... | inetd ... | gc ...)

Subcommands:
  syslog
    Opens a Unix socket listening on /dev/log, reads messages from it, and dumps
    them to stdout.

  inetd PORT [EXE [ARGS...]]
    Opens a socket listening on PORT, and on connection invokes the executable
    EXE with any provided ARGS. By default EXE is /usr/sbin/approx and ARGS is
    empty.

  gc RUN-INTERVAL MAX-AGE [CACHE-DIR]
    Every RUN-INTERVAL, this checks for any files older than MAX-AGE in
    CACHE-DIR (recursively) and deletes them. Both RUN-INTERVAL and MAX-AGE are
    given as colon-separated list of day, hour, minute, and second units.
    Missing values are assumed to be most-significant:

        30:12:10:08 (30 days, 12 hours, 10 minutes, 8 seconds)
        12:10:08 (12 hours, 10 minutes, 8 seconds)
        15 (15 seconds)

    By default CACHE-DIR is /var/cache/approx.
",
        message
    );
    exit(1)
}

fn main() {
    let mut argv = env::args().skip(1);
    match argv.next().as_deref() {
        Some("syslog") => dumb_syslog(),
        Some("inetd") => {
            let port = match argv.next().map(|p| u16::from_str_radix(&p, 10)) {
                Some(Ok(p)) => p,
                Some(Err(err)) => usage(format!("Could not parse port number: {0}", err)),
                None => usage("inetd command must have a port"),
            };

            let mut command = Vec::new();
            while let Some(c) = argv.next() {
                command.push(c)
            }
            if command.len() == 0 {
                command.push("/usr/sbin/approx".to_string());
            }

            dumb_inetd(port, command)
        }
        Some("gc") => {
            let check_interval = match argv.next().map(|d| parse_duration(&d)) {
                Some(Ok(d)) => d,
                Some(Err(err)) => usage(format!("Could not parse RUN-INTERVAL: {0}", err)),
                None => usage("gc command must have a run interval"),
            };

            let max_age = match argv.next().map(|d| parse_duration(&d)) {
                Some(Ok(d)) => d,
                Some(Err(err)) => usage(format!("Could not parse MAX-AGE: {0}", err)),
                None => usage("gc command must have a max-age"),
            };

            let dir = match argv.next() {
                Some(d) => d,
                None => "/var/cache/approx".to_string(),
            };

            garbage_collect_cache(check_interval, max_age, &dir)
        }
        Some(cmd) => usage(format!("Unknown command: {0}", cmd)),
        _ => usage("No command was provided"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_parse_duration_secs() {
        assert_eq!(Ok(Duration::from_secs(27)), parse_duration("27"));
    }

    #[test]
    pub fn test_parse_duration_secs_to_mins() {
        assert_eq!(
            Ok(Duration::from_secs(9 * 60 + 27)),
            parse_duration("09:27")
        );
    }

    #[test]
    pub fn test_parse_duration_secs_to_hours() {
        assert_eq!(
            Ok(Duration::from_secs(13 * 3600 + 9 * 60 + 27)),
            parse_duration("13:09:27")
        );
    }

    #[test]
    pub fn test_parse_duration_secs_to_days() {
        assert_eq!(
            Ok(Duration::from_secs(4 * 86400 + 13 * 3600 + 9 * 60 + 27)),
            parse_duration("04:13:09:27")
        );
    }
}
