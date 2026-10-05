//! `hash-password`: the Argon2id PHC string for `auth.password_hash`.

use argon2::{Argon2, PasswordHasher};
use std::io::{BufRead, IsTerminal};

/// Argon2id with default parameters and a random salt, as a PHC string.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!password.is_empty(), "password must not be empty");
    let hash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("hashing failed: {e}"))?;
    Ok(hash.to_string())
}

/// The password from a prompt without echo, asked twice, or the first stdin line when piped.
pub fn read_password() -> anyhow::Result<String> {
    if std::io::stdin().is_terminal() {
        let first = rpassword::prompt_password("Password: ")?;
        let second = rpassword::prompt_password("Repeat password: ")?;
        return confirmed(first, second);
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

fn confirmed(first: String, second: String) -> anyhow::Result<String> {
    anyhow::ensure!(first == second, "passwords do not match");
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{PasswordHash, PasswordVerifier};

    #[test]
    fn hash_of_known_password_verifies() {
        let phc = hash_password("secret").unwrap();
        assert!(phc.starts_with("$argon2id$v=19$"), "{phc}");
        let parsed = PasswordHash::new(&phc).unwrap();
        assert!(
            Argon2::default()
                .verify_password(b"secret", &parsed)
                .is_ok()
        );
        assert!(
            Argon2::default()
                .verify_password(b"Secret", &parsed)
                .is_err()
        );
        // A fresh salt each time.
        assert_ne!(phc, hash_password("secret").unwrap());
        assert!(hash_password("").is_err());
    }

    #[test]
    fn prompt_needs_matching_repeat() {
        assert_eq!(confirmed("a".into(), "a".into()).unwrap(), "a");
        let e = confirmed("a".into(), "b".into()).unwrap_err().to_string();
        assert_eq!(e, "passwords do not match");
    }
}
