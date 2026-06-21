use crate::classifier::{classify, Protection};
use nix::sys::signal::{kill as nix_kill, Signal};
use nix::unistd::Pid;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum KillError {
    Protected,
    PermissionDenied,
    NotFound,
    Other,
}

/// Returns true if the process exists (including as a zombie).
pub fn is_alive(pid: u32) -> bool {
    // Signal 0 performs error checking without sending a signal.
    match nix_kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(nix::errno::Errno::EPERM) => true, // exists, but not ours
        Err(_) => false,
    }
}

/// Send SIGTERM; if still alive after 2s, send SIGKILL. Refuses protected pids.
pub fn kill(pid: u32, name: &str) -> Result<(), KillError> {
    if classify(name, pid) == Protection::Protected {
        return Err(KillError::Protected);
    }

    let target = Pid::from_raw(pid as i32);
    match nix_kill(target, Signal::SIGTERM) {
        Ok(()) => {}
        Err(nix::errno::Errno::ESRCH) => return Err(KillError::NotFound),
        Err(nix::errno::Errno::EPERM) => return Err(KillError::PermissionDenied),
        Err(_) => return Err(KillError::Other),
    }

    // Escalate in the background so the UI never blocks.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        if is_alive(pid) {
            let _ = nix_kill(target, Signal::SIGKILL);
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::wait::{waitpid, WaitPidFlag};
    use std::process::Command;
    use std::thread::sleep;

    fn spawn_sleeper() -> u32 {
        Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep")
            .id()
    }

    /// Reap a child so it leaves zombie state and signal-0 returns ESRCH.
    fn reap(pid: u32) {
        let _ = waitpid(Pid::from_raw(pid as i32), Some(WaitPidFlag::WNOHANG));
        sleep(Duration::from_millis(50));
        let _ = waitpid(Pid::from_raw(pid as i32), None);
    }

    #[test]
    fn is_alive_true_for_running_then_false_after_kill() {
        let pid = spawn_sleeper();
        assert!(is_alive(pid), "sleeper should be alive");
        nix_kill(Pid::from_raw(pid as i32), Signal::SIGKILL).unwrap();
        sleep(Duration::from_millis(300));
        reap(pid);
        assert!(!is_alive(pid), "sleeper should be dead after SIGKILL");
    }

    #[test]
    fn kill_refuses_protected() {
        // pid < 100 is protected regardless of liveness
        assert_eq!(kill(1, "launchd"), Err(KillError::Protected));
    }

    #[test]
    fn kill_terminates_killable_process() {
        let pid = spawn_sleeper();
        assert_eq!(kill(pid, "sleep"), Ok(()));
        sleep(Duration::from_millis(500));
        reap(pid);
        assert!(!is_alive(pid), "sleeper should be terminated by SIGTERM");
    }

    #[test]
    fn kill_not_found_for_dead_pid() {
        let pid = spawn_sleeper();
        nix_kill(Pid::from_raw(pid as i32), Signal::SIGKILL).unwrap();
        sleep(Duration::from_millis(300));
        // reaping note: child may be a zombie; use a clearly-unused high pid instead
        assert_eq!(kill(999_999, "ghost"), Err(KillError::NotFound));
    }
}
