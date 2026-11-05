//! The closed list of built-in icon names (§6.3). `icon:` validates against
//! it; the page's SVG sprite (web/, M4) has exactly these symbols, which a test
//! there asserts.

pub const ICONS: &[&str] = &[
    "activity", "alert", "archive", "bell", "book", "box", "bug", "calendar", "chart", "check",
    "clock", "cloud", "code", "compass", "cpu", "database", "dollar", "file", "flag", "folder",
    "git", "globe", "grid", "heart", "help", "home", "key", "layers", "link", "list", "lock",
    "mail", "map", "message", "monitor", "package", "phone", "refresh", "rocket", "search",
    "server", "settings", "shield", "star", "terminal", "ticket", "tool", "users", "video", "zap",
];

pub fn is_icon(name: &str) -> bool {
    ICONS.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sorted_unique_and_covers_the_spec_examples() {
        let mut v = super::ICONS.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v, super::ICONS, "keep ICONS sorted and unique");
        for name in [
            "refresh", "phone", "users", "grid", "globe", "server", "ticket", "cloud", "file",
            "message", "chart", "shield", "key", "rocket", "box", "database", "git",
        ] {
            assert!(super::is_icon(name), "{name}");
        }
    }
}
