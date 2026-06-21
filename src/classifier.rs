#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Protection {
    Killable,
    Protected,
}

const PROTECTED_NAMES: &[&str] = &[
    "WindowServer",
    "kernel_task",
    "launchd",
    "loginwindow",
    "WindowManager",
    "coreaudiod",
    "mds",
    "mds_stores",
];

pub fn classify(name: &str, pid: u32) -> Protection {
    if pid < 100 {
        return Protection::Protected;
    }
    if PROTECTED_NAMES.contains(&name) {
        return Protection::Protected;
    }
    Protection::Killable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_known_system_names() {
        for name in [
            "WindowServer",
            "kernel_task",
            "launchd",
            "loginwindow",
            "WindowManager",
            "coreaudiod",
            "mds",
            "mds_stores",
        ] {
            assert_eq!(classify(name, 5000), Protection::Protected, "{name}");
        }
    }

    #[test]
    fn protects_low_pids() {
        assert_eq!(classify("anything", 1), Protection::Protected);
        assert_eq!(classify("anything", 99), Protection::Protected);
    }

    #[test]
    fn allows_normal_user_process() {
        assert_eq!(classify("python", 8981), Protection::Killable);
        assert_eq!(classify("Google Chrome for Testing", 8360), Protection::Killable);
    }

    #[test]
    fn pid_100_is_killable() {
        assert_eq!(classify("python", 100), Protection::Killable);
    }
}
