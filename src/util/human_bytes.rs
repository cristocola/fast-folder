//! The one byte formatter.
//!
//! Two of these existed: the browser's Size cell counted up to terabytes, and
//! `template show`'s bundle summary stopped at gigabytes and rounded from the
//! other direction. A project library and a template bundle are measured in the
//! same units, so they are printed by the same function.

/// Render a byte count the way every fastf surface prints one: whole bytes
/// below a kilobyte, one decimal place above it, binary units throughout.
pub fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;

    let bytes_f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes_f < MIB {
        format!("{:.1} KB", bytes_f / KIB)
    } else if bytes_f < GIB {
        format!("{:.1} MB", bytes_f / MIB)
    } else if bytes_f < TIB {
        format!("{:.1} GB", bytes_f / GIB)
    } else {
        format!("{:.1} TB", bytes_f / TIB)
    }
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn covers_bytes_through_terabytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1024_u64.pow(2)), "1.0 MB");
        assert_eq!(human_bytes(1024_u64.pow(3)), "1.0 GB");
        assert_eq!(human_bytes(1024_u64.pow(4)), "1.0 TB");
    }

    /// The boundary the two old formatters disagreed on: one switched unit at
    /// `>= MB`, the other at `< MB`. They agree here by construction now.
    #[test]
    fn the_unit_boundary_is_the_unit_itself() {
        assert_eq!(human_bytes(1024_u64.pow(2) - 1), "1024.0 KB");
        assert_eq!(human_bytes(1024_u64.pow(2)), "1.0 MB");
    }
}
