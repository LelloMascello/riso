//! TUI: pseudo-terminal running the `ssh` client of a session.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::Sender;

use super::event::AppEvent;

pub struct Pty {
    master: OwnedFd,
    writer: File,
    child: Child,
}

fn winsize(rows: u16, cols: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Pty {
    /// Spawns `argv` on a new PTY of the given size. Output is delivered as
    /// [`AppEvent::PtyOutput`]; [`AppEvent::PtyClosed`] follows the end of
    /// the stream.
    pub fn spawn(
        argv: &[String],
        rows: u16,
        cols: u16,
        session: u64,
        tx: Sender<AppEvent>,
    ) -> io::Result<Pty> {
        let Some((program, args)) = argv.split_first() else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "comando vuoto"));
        };

        let (mut master_fd, mut slave_fd) = (-1, -1);
        let ws = winsize(rows, cols);
        // SAFETY: valid out-pointers; name/termios are optional (NULL).
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &ws,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openpty succeeded, both descriptors are fresh and owned here.
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master_fd), OwnedFd::from_raw_fd(slave_fd)) };
        // SAFETY: plain fcntl on an owned descriptor.
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };

        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave))
            .env("TERM", "xterm-256color")
            .env_remove("COLUMNS")
            .env_remove("LINES");
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        // Drop the slave copies held by `cmd`, so the reader sees EOF/EIO
        // once the child exits.
        drop(cmd);

        let mut reader = File::from(master.try_clone()?);
        let writer = File::from(master.try_clone()?);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(AppEvent::PtyOutput(session, buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = tx.send(AppEvent::PtyClosed(session));
        });

        Ok(Pty { master, writer, child })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let ws = winsize(rows, cols);
        // SAFETY: TIOCSWINSZ reads a winsize struct from a valid pointer.
        unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) };
    }

    /// Reaps the child after the output stream ended.
    pub fn wait(&mut self) -> Option<ExitStatus> {
        self.child.wait().ok()
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}
