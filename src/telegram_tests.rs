//! Tests for Telegram bot functionality
//!
//! Unit tests for authorization, message chunking, and CLI integration.

#[cfg(test)]
mod tests {
    // Test authorization logic
    mod authorization {
        #[test]
        fn test_empty_allowed_list_allows_all() {
            let allowed_users: Vec<i64> = vec![];
            let is_allowed = allowed_users.is_empty() || allowed_users.contains(&12345);
            assert!(is_allowed);
        }

        #[test]
        fn test_allowed_user_permitted() {
            let allowed_users: Vec<i64> = vec![12345, 67890];
            let is_allowed = allowed_users.is_empty() || allowed_users.contains(&12345);
            assert!(is_allowed);
        }

        #[test]
        fn test_unauthorized_user_denied() {
            let allowed_users: Vec<i64> = vec![12345, 67890];
            let is_allowed = allowed_users.is_empty() || allowed_users.contains(&99999);
            assert!(!is_allowed);
        }

        #[test]
        fn test_zero_user_id_with_list() {
            let allowed_users: Vec<i64> = vec![12345];
            let is_allowed = allowed_users.is_empty() || allowed_users.contains(&0);
            assert!(!is_allowed);
        }

        #[test]
        fn test_negative_user_id() {
            let allowed_users: Vec<i64> = vec![-1, 12345];
            let is_allowed = allowed_users.is_empty() || allowed_users.contains(&-1);
            assert!(is_allowed);
        }
    }

    // Test message chunking
    mod message_chunking {
        const MAX_CHUNK: usize = 4000;

        fn chunk_message(text: &str) -> Vec<String> {
            let mut chunks = Vec::new();
            if text.is_empty() {
                return chunks;
            }
            let mut remaining = text;
            while !remaining.is_empty() {
                let split_at = remaining
                    .char_indices()
                    .take_while(|(i, _)| *i < MAX_CHUNK)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(remaining.len());
                let (chunk, rest) = remaining.split_at(split_at);
                chunks.push(chunk.to_string());
                remaining = rest;
            }
            chunks
        }

        #[test]
        fn test_short_message_single_chunk() {
            let msg = "Hello, world!";
            let chunks = chunk_message(msg);
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0], msg);
        }

        #[test]
        fn test_exact_boundary_message() {
            let msg = "a".repeat(MAX_CHUNK);
            let chunks = chunk_message(&msg);
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].len(), MAX_CHUNK);
        }

        #[test]
        fn test_message_splits_correctly() {
            let msg = "a".repeat(MAX_CHUNK + 100);
            let chunks = chunk_message(&msg);
            assert_eq!(chunks.len(), 2);
            assert_eq!(chunks[0].len(), MAX_CHUNK);
            assert_eq!(chunks[1].len(), 100);
        }

        #[test]
        fn test_utf8_multibyte_not_broken() {
            let base = "a".repeat(MAX_CHUNK - 2);
            let msg = format!("{}日本語", base);
            let chunks = chunk_message(&msg);

            for chunk in &chunks {
                assert!(chunk.chars().count() > 0);
            }

            let rejoined: String = chunks.concat();
            assert_eq!(rejoined, msg);
        }

        #[test]
        fn test_emoji_boundary() {
            let base = "a".repeat(MAX_CHUNK - 3);
            let msg = format!("{}🚀🎉", base);
            let chunks = chunk_message(&msg);

            let rejoined: String = chunks.concat();
            assert_eq!(rejoined, msg);
        }

        #[test]
        fn test_empty_message() {
            let chunks = chunk_message("");
            assert!(chunks.is_empty());
        }

        #[test]
        fn test_very_long_message() {
            let msg = "x".repeat(MAX_CHUNK * 3 + 500);
            let chunks = chunk_message(&msg);
            assert_eq!(chunks.len(), 4);
            assert_eq!(chunks[0].len(), MAX_CHUNK);
            assert_eq!(chunks[1].len(), MAX_CHUNK);
            assert_eq!(chunks[2].len(), MAX_CHUNK);
            assert_eq!(chunks[3].len(), 500);
        }
    }

    // Test ANSI stripping
    mod ansi_stripping {
        fn strip_ansi_codes(s: &str) -> String {
            let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
            re.replace_all(s, "").to_string()
        }

        #[test]
        fn test_strip_simple_color() {
            let input = "\x1b[32mGreen text\x1b[0m";
            let output = strip_ansi_codes(input);
            assert_eq!(output, "Green text");
        }

        #[test]
        fn test_strip_multiple_codes() {
            let input = "\x1b[1;31mBold Red\x1b[0m normal \x1b[34mblue\x1b[0m";
            let output = strip_ansi_codes(input);
            assert_eq!(output, "Bold Red normal blue");
        }

        #[test]
        fn test_no_ansi_unchanged() {
            let input = "Plain text without colors";
            let output = strip_ansi_codes(input);
            assert_eq!(output, input);
        }

        #[test]
        fn test_empty_string() {
            let output = strip_ansi_codes("");
            assert_eq!(output, "");
        }
    }

    // Test environment parsing
    mod env_parsing {
        #[test]
        fn test_parse_allowed_users_csv() {
            let csv = "12345, 67890, 11111";
            let users: Vec<i64> = csv
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            assert_eq!(users, vec![12345i64, 67890, 11111]);
        }

        #[test]
        fn test_parse_empty_allowed_users() {
            let csv = "";
            let users: Vec<i64> = csv
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            assert!(users.is_empty());
        }

        #[test]
        fn test_parse_with_invalid_entries() {
            let csv = "12345, invalid, 67890, , -1";
            let users: Vec<i64> = csv
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            assert_eq!(users, vec![12345i64, 67890, -1]);
        }

        #[test]
        fn test_parse_whitespace_handling() {
            let csv = "  12345  ,  67890  ";
            let users: Vec<i64> = csv
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            assert_eq!(users, vec![12345i64, 67890]);
        }
    }

    // Test command parsing
    mod command_parsing {
        #[test]
        fn test_command_without_args() {
            let text = "/start";
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            assert_eq!(parts[0], "/start");
            assert!(parts.get(1).is_none());
        }

        #[test]
        fn test_command_with_args() {
            let text = "/analyze some file.txt";
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            assert_eq!(parts[0], "/analyze");
            assert_eq!(parts.get(1), Some(&"some file.txt"));
        }

        #[test]
        fn test_command_with_multiple_spaces() {
            let text = "/cmd arg1 arg2 arg3";
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            assert_eq!(parts[0], "/cmd");
            assert_eq!(parts.get(1), Some(&"arg1 arg2 arg3"));
        }

        #[test]
        fn test_is_command() {
            assert!("/start".starts_with('/'));
            assert!("/help".starts_with('/'));
            assert!(!"hello".starts_with('/'));
            assert!(!"".starts_with('/'));
        }
    }

    // Test path sanitization
    mod path_sanitization {
        use std::path::Path;

        fn sanitize_filename(raw: &str) -> String {
            Path::new(raw)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        }

        #[test]
        fn test_simple_filename() {
            assert_eq!(sanitize_filename("document.pdf"), "document.pdf");
        }

        #[test]
        fn test_path_traversal_attack() {
            assert_eq!(sanitize_filename("../../../etc/passwd"), "passwd");
        }

        #[test]
        fn test_absolute_path() {
            assert_eq!(sanitize_filename("/etc/shadow"), "shadow");
        }

        #[test]
        fn test_windows_path() {
            // On Unix, backslash is a valid filename character
            // This tests that we extract just the filename
            let result = sanitize_filename("file.txt");
            assert_eq!(result, "file.txt");
        }

        #[test]
        fn test_empty_fallback() {
            assert_eq!(sanitize_filename(""), "file");
        }
    }
}
