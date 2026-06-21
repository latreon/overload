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

/// Pure helper: returns true only if the process is still alive AND still
/// classifies as Killable. Called before the deferred SIGKILL to guard
/// against PID reuse — a recycled PID may belong to a protected process.
pub fn should_sigkill(pid: u32, name: &str) -> bool {
    is_alive(pid) && classify(name, pid) == Protection::Killable
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
    // We capture `name` as an owned String so we can re-classify inside the
    // thread. PIDs can be recycled within the 2-second sleep window: if the
    // original process exited and a new protected process inherited that PID,
    // sending SIGKILL without re-checking would violate the protection invariant.
    let owned_name = name.to_owned();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        if should_sigkill(pid, &owned_name) {
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

    // --- should_sigkill pure-logic tests ---

    #[test]
    fn should_sigkill_false_for_protected_name() {
        // A protected name must never be escalated, regardless of liveness.
        // We use pid 5000 (well above 100) so the low-pid guard doesn't fire —
        // only the name-based protection should block it.
        assert!(!should_sigkill(5000, "launchd"));
        assert!(!should_sigkill(5000, "kernel_task"));
    }

    #[test]
    fn should_sigkill_false_for_dead_pid() {
        // pid 999_999 is virtually guaranteed to not exist.
        assert!(!should_sigkill(999_999, "ghost"));
    }

    #[test]
    fn should_sigkill_false_for_low_pid_regardless_of_name() {
        // Pids < 100 are always protected by classify().
        assert!(!should_sigkill(1, "sleep"));
    }

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
