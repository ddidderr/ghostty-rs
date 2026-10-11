#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LocalHostnameValidationError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("unexpected error")]
    Unexpected,
}

/// Validates a hostname according to [RFC 1123](https://www.rfc-editor.org/rfc/rfc1123)
///
/// A permissive validator would consider strings like ".example.com",
/// "exa..mple.com", and "-example.com" to be valid hostnames, which
/// is incorrect.
pub fn is_valid(hostname: &str) -> bool {
    let hostname = hostname.as_bytes();
    if hostname.is_empty() {
        return false;
    }
    if hostname[0] == b'.' {
        return false;
    }

    // Ignore trailing dot (FQDN). It doesn't count toward our length.
    let end = if hostname[hostname.len() - 1] == b'.' {
        if hostname.len() == 1 {
            return false;
        }
        hostname.len() - 1
    } else {
        hostname.len()
    };

    if end > 253 {
        return false;
    }

    // Hostnames are divided into dot-separated "labels", which:
    //
    // - Start with a letter or digit
    // - Can contain letters, digits, or hyphens
    // - Must end with a letter or digit
    // - Have a minimum of 1 character and a maximum of 63
    let mut label_start = 0usize;
    let mut label_len = 0usize;
    for (i, &c) in hostname[..end].iter().enumerate() {
        match c {
            b'.' => {
                if label_len == 0 || label_len > 63 {
                    return false;
                }
                if !hostname[label_start].is_ascii_alphanumeric() {
                    return false;
                }
                if !hostname[i - 1].is_ascii_alphanumeric() {
                    return false;
                }

                label_start = i + 1;
                label_len = 0;
            }
            b'-' => {
                label_len += 1;
            }
            _ => {
                if !c.is_ascii_alphanumeric() {
                    return false;
                }
                label_len += 1;
            }
        }
    }

    // Validate the final label
    if label_len == 0 || label_len > 63 {
        return false;
    }
    if !hostname[label_start].is_ascii_alphanumeric() {
        return false;
    }
    if !hostname[end - 1].is_ascii_alphanumeric() {
        return false;
    }

    true
}

/// The current machine's hostname (`gethostname`).
fn our_hostname() -> String {
    rustix::system::uname()
        .nodename()
        .to_string_lossy()
        .into_owned()
}

/// Checks if a hostname is local to the current machine. This matches
/// both "localhost" and the current hostname of the machine (as returned
/// by `gethostname`).
pub fn is_local(hostname: &str) -> Result<bool, LocalHostnameValidationError> {
    // A 'localhost' hostname is always considered local.
    if hostname == "localhost" {
        return Ok(true);
    }

    // If hostname is not "localhost" it must match our hostname.
    Ok(hostname == our_hostname())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_hostnames() {
        // Valid hostnames
        assert!(is_valid("example"));
        assert!(is_valid("example.com"));
        assert!(is_valid("www.example.com"));
        assert!(is_valid("sub.domain.example.com"));
        assert!(is_valid("example.com."));
        assert!(is_valid("host-name.example.com."));
        assert!(is_valid("123.example.com."));
        assert!(is_valid("a-b.com"));
        assert!(is_valid("a.b.c.d.e.f.g"));
        assert!(is_valid("127.0.0.1")); // Also a valid hostname
        assert!(is_valid(&format!("{}.com", "a".repeat(63)))); // Label exactly 63 chars (valid)
        assert!(is_valid(&format!("{}a", "a.".repeat(126)))); // Total length 253 (valid)

        // Invalid hostnames
        assert!(!is_valid(""));
        assert!(!is_valid(".example.com"));
        assert!(!is_valid("example.com.."));
        assert!(!is_valid("host..domain"));
        assert!(!is_valid("-hostname"));
        assert!(!is_valid("hostname-"));
        assert!(!is_valid("a.-.b"));
        assert!(!is_valid("host_name.com"));
        assert!(!is_valid("."));
        assert!(!is_valid(".."));
        assert!(!is_valid(&format!("{}.com", "a".repeat(64)))); // Label length 64 (too long)
        assert!(!is_valid(&format!("{}ab", "a.".repeat(126)))); // Total length 254 (too long)
    }

    #[test]
    fn is_local_returns_true_when_provided_hostname_is_localhost() {
        assert!(is_local("localhost").unwrap());
    }

    #[test]
    fn is_local_returns_true_when_hostname_is_local() {
        let local = our_hostname();
        assert!(is_local(&local).unwrap());
    }

    #[test]
    fn is_local_returns_false_when_hostname_is_not_local() {
        assert!(!is_local("not-the-local-hostname").unwrap());
    }
}
