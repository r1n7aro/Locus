/// Incremental UTF-8 decoder for byte streams whose transport chunks may end
/// in the middle of a multibyte code point. Invalid complete sequences retain
/// the previous lossy-decoding behavior; incomplete suffixes wait for the next
/// chunk instead of becoming U+FFFD.
#[derive(Debug, Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut output = String::new();

        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        let valid = std::str::from_utf8(&self.pending[..valid_up_to])
                            .expect("validated UTF-8 prefix");
                        output.push_str(valid);
                        self.pending.drain(..valid_up_to);
                    }
                    let Some(error_len) = error.error_len() else {
                        break;
                    };
                    output.push('\u{fffd}');
                    self.pending.drain(..error_len.min(self.pending.len()));
                }
            }
        }

        output
    }

    pub fn finish(&mut self) -> String {
        let output = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        output
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8StreamDecoder;

    #[test]
    fn preserves_multibyte_characters_split_across_chunks() {
        let source = "中文\u{e200}cite\u{e202}turn1view0\u{e201}".as_bytes();
        let mut decoder = Utf8StreamDecoder::default();
        let mut decoded = String::new();
        for byte in source {
            decoded.push_str(&decoder.push(std::slice::from_ref(byte)));
        }
        decoded.push_str(&decoder.finish());
        assert_eq!(decoded, "中文\u{e200}cite\u{e202}turn1view0\u{e201}");
    }

    #[test]
    fn replaces_complete_invalid_sequences_and_keeps_following_text() {
        let mut decoder = Utf8StreamDecoder::default();
        assert_eq!(decoder.push(b"a\xffb"), "a\u{fffd}b");
        assert_eq!(decoder.finish(), "");
    }
}
