//! Fixed-storage W3C version-00 trace context. No exporter, allocator or globals.
//!
//! Invalid parents discard the entire context; invalid state discards only state.
//! Baggage has no API here. HTTP field ownership remains the caller's concern.

#![no_std]

pub const PARENT_BYTES: usize = 55;
pub const STATE_BYTES: usize = 512;
pub const STATE_MEMBERS: usize = 32;

#[derive(Debug, Eq, PartialEq)]
pub struct Parent {
    pub trace: [u8; 16],
    pub span: [u8; 8],
    pub sampled: bool,
}

impl Parent {
    /// Preserve the original runtime's strict version, length, lowercase and
    /// nonzero-ID rules. Unknown flag bits do not change the sampled decision.
    pub const fn parse(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != PARENT_BYTES
            || bytes[0] != b'0'
            || bytes[1] != b'0'
            || bytes[2] != b'-'
            || bytes[35] != b'-'
            || bytes[52] != b'-'
        {
            return None;
        }
        let trace = match decode::<16>(bytes, 3) {
            Some(value) => value,
            None => return None,
        };
        let span = match decode::<8>(bytes, 36) {
            Some(value) => value,
            None => return None,
        };
        let flags = match decode::<1>(bytes, 53) {
            Some(value) => value[0],
            None => return None,
        };
        if !nonzero(&trace) || !nonzero(&span) {
            return None;
        }
        Some(Self {
            trace,
            span,
            sampled: flags & 1 == 1,
        })
    }
}

const fn nonzero(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0 {
            return true;
        }
        index += 1;
    }
    false
}

const fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

const fn decode<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    let mut result = [0; N];
    let mut index = 0;
    while index < N {
        let high = match digit(bytes[offset + index * 2]) {
            Some(value) => value,
            None => return None,
        };
        let low = match digit(bytes[offset + index * 2 + 1]) {
            Some(value) => value,
            None => return None,
        };
        result[index] = high * 16 + low;
        index += 1;
    }
    Some(result)
}

const fn hex_digit(value: u8) -> u8 {
    if value < 10 {
        b'0' + value
    } else {
        b'a' + value - 10
    }
}

fn encode(bytes: &[u8], output: &mut [u8]) {
    for (byte, pair) in bytes.iter().zip(output.as_chunks_mut::<2>().0) {
        pair[0] = hex_digit(byte >> 4);
        pair[1] = hex_digit(byte & 15);
    }
}

/// A validated borrowed state. Empty list members and surrounding HTTP OWS
/// are accepted; output is canonicalized without empty members or OWS.
pub struct State<'a> {
    raw: &'a str,
}

impl<'a> State<'a> {
    pub fn parse(raw: &'a str) -> Option<Self> {
        if raw.len() > STATE_BYTES {
            return None;
        }
        let mut keys = [""; STATE_MEMBERS];
        let mut used = 0;
        for (member_index, member) in raw.split(',').enumerate() {
            if member_index >= STATE_MEMBERS {
                return None;
            }
            let member = member.trim_matches([' ', '\t']);
            if member.is_empty() {
                continue;
            }
            let (key, value) = member.split_once('=')?;
            if !valid_key(key) || !valid_value(value) || keys[..used].contains(&key) {
                return None;
            }
            keys[used] = key;
            used += 1;
        }
        Some(Self { raw })
    }

    pub fn pairs(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.raw
            .split(',')
            .filter_map(|member| member.trim_matches([' ', '\t']).split_once('='))
    }

    fn write(&self, output: &mut [u8; STATE_BYTES]) -> usize {
        let mut length = 0;
        for (key, value) in self.pairs() {
            if length != 0 {
                output[length] = b',';
                length += 1;
            }
            output[length..length + key.len()].copy_from_slice(key.as_bytes());
            length += key.len();
            output[length] = b'=';
            length += 1;
            output[length..length + value.len()].copy_from_slice(value.as_bytes());
            length += value.len();
        }
        length
    }
}

fn key_tail(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'*' | b'/'))
}

fn valid_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 256 {
        return false;
    }
    if let Some((tenant, system)) = key.split_once('@') {
        !tenant.is_empty()
            && tenant.len() <= 241
            && !system.is_empty()
            && system.len() <= 14
            && (tenant.as_bytes()[0].is_ascii_lowercase() || tenant.as_bytes()[0].is_ascii_digit())
            && system.as_bytes()[0].is_ascii_lowercase()
            && key_tail(tenant.as_bytes())
            && key_tail(system.as_bytes())
    } else {
        key.as_bytes()[0].is_ascii_lowercase() && key_tail(key.as_bytes())
    }
}

fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.ends_with(' ')
        && value
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, 0x20..=0x7e) && *byte != b',' && *byte != b'=')
}

/// Owned, bounded transport storage. Copies into this storage are explicit;
/// callers can forward it without cloning Strings or allocating a header map.
pub struct Carrier {
    parent: [u8; PARENT_BYTES],
    state: [u8; STATE_BYTES],
    state_length: usize,
    present: bool,
}

impl Default for Carrier {
    fn default() -> Self {
        Self::empty()
    }
}

impl Carrier {
    pub const fn empty() -> Self {
        Self {
            parent: [0; PARENT_BYTES],
            state: [0; STATE_BYTES],
            state_length: 0,
            present: false,
        }
    }

    pub fn from_parts(parent: Option<&str>, state: Option<&str>) -> Self {
        Self::from_headers(parent.map(Some).into_iter(), state.map(Some).into_iter())
    }

    /// Each iterator item is one field. None means a present field could not be
    /// decoded as header text. Repeated parents invalidate the whole context;
    /// repeated state fields are joined in wire order before one validation.
    pub fn from_headers<'a>(
        mut parents: impl Iterator<Item = Option<&'a str>>,
        states: impl Iterator<Item = Option<&'a str>>,
    ) -> Self {
        let mut carrier = Self::empty();
        let Some(Some(parent)) = parents.next() else {
            return carrier;
        };
        if parents.next().is_some() || Parent::parse(parent).is_none() {
            return carrier;
        }
        carrier.parent.copy_from_slice(parent.as_bytes());
        carrier.present = true;
        let mut joined = [0; STATE_BYTES];
        let mut length = 0;
        for (index, field) in states.enumerate() {
            let Some(field) = field else {
                return carrier;
            };
            if index >= STATE_MEMBERS {
                return carrier;
            }
            let delimiter = usize::from(index != 0);
            if field.len() > STATE_BYTES - length || delimiter > STATE_BYTES - length - field.len()
            {
                return carrier;
            }
            if delimiter != 0 {
                joined[length] = b',';
                length += 1;
            }
            joined[length..length + field.len()].copy_from_slice(field.as_bytes());
            length += field.len();
        }
        if let Ok(raw) = core::str::from_utf8(&joined[..length])
            && let Some(state) = State::parse(raw)
        {
            carrier.state_length = state.write(&mut carrier.state);
        }
        carrier
    }

    /// Replace a validated incoming carrier with an actual local span context.
    /// SDK span IDs are explicit small-value transfers, not payload copies.
    /// An invalid state is dropped while the valid parent remains available.
    pub fn set_span<'a>(
        &mut self,
        trace: &[u8; 16],
        span: &[u8; 8],
        sampled: bool,
        pairs: impl Iterator<Item = (&'a str, &'a str)>,
    ) {
        self.present = false;
        self.state_length = 0;
        if !nonzero(trace) || !nonzero(span) {
            return;
        }
        self.parent[..3].copy_from_slice(b"00-");
        encode(trace, &mut self.parent[3..35]);
        self.parent[35] = b'-';
        encode(span, &mut self.parent[36..52]);
        self.parent[52] = b'-';
        self.parent[53] = b'0';
        self.parent[54] = if sampled { b'1' } else { b'0' };
        self.present = true;
        let mut length = 0;
        for (index, (key, value)) in pairs.enumerate() {
            if index >= STATE_MEMBERS || !valid_key(key) || !valid_value(value) {
                return;
            }
            let additional = usize::from(index != 0) + key.len() + 1 + value.len();
            if additional > STATE_BYTES - length {
                return;
            }
            if index != 0 {
                self.state[length] = b',';
                length += 1;
            }
            self.state[length..length + key.len()].copy_from_slice(key.as_bytes());
            length += key.len();
            self.state[length] = b'=';
            length += 1;
            self.state[length..length + value.len()].copy_from_slice(value.as_bytes());
            length += value.len();
        }
        let raw = core::str::from_utf8(&self.state[..length]).expect("validated ASCII state");
        if State::parse(raw).is_some() {
            self.state_length = length;
        }
    }

    pub fn parent(&self) -> Option<&str> {
        self.present
            .then(|| core::str::from_utf8(&self.parent).expect("validated ASCII parent"))
    }

    pub fn state(&self) -> Option<&str> {
        (self.present && self.state_length != 0).then(|| {
            core::str::from_utf8(&self.state[..self.state_length]).expect("validated ASCII state")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static PARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn frozen_cases_are_shared_with_the_java_process_hook() {
        fn decode<'a>(field: &str, output: &'a mut [u8; 1024]) -> Option<&'a str> {
            if field == "-" {
                return None;
            }
            let mut bytes = field.bytes();
            let mut used = 0;
            while let Some(byte) = bytes.next() {
                output[used] = if byte == b'\\' {
                    match bytes.next().expect("escaped test byte") {
                        b't' => b'\t',
                        b'r' => b'\r',
                        b'n' => b'\n',
                        _ => panic!("unknown fixture escape"),
                    }
                } else {
                    byte
                };
                used += 1;
            }
            Some(core::str::from_utf8(&output[..used]).expect("UTF-8 fixture"))
        }
        let mut cases = 0;
        for line in include_str!("../cases.tsv")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let mut fields = line.split('\t');
            let name = fields.next().expect("case name");
            let mut parent = [0; 1024];
            let mut state = [0; 1024];
            let mut expected_parent = [0; 1024];
            let mut expected_state = [0; 1024];
            let carrier = Carrier::from_parts(
                decode(fields.next().expect("parent"), &mut parent),
                decode(fields.next().expect("state"), &mut state),
            );
            assert_eq!(
                carrier.parent(),
                decode(
                    fields.next().expect("expected parent"),
                    &mut expected_parent
                ),
                "{name}"
            );
            assert_eq!(
                carrier.state(),
                decode(fields.next().expect("expected state"), &mut expected_state),
                "{name}"
            );
            assert!(fields.next().is_none());
            cases += 1;
        }
        assert_eq!(cases, 23);
    }

    #[test]
    fn original_parent_rules_and_flag_mask_are_preserved() {
        let parent = Parent::parse(PARENT).expect("W3C example");
        assert!(parent.sampled);
        for invalid in [
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
        ] {
            assert!(Parent::parse(invalid).is_none());
        }
        assert!(
            Parent::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-ff")
                .expect("unknown flags ignored")
                .sampled
        );
        assert!(
            !Parent::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-fe")
                .expect("unknown flags ignored")
                .sampled
        );
    }

    #[test]
    fn repeated_state_fields_are_combined_and_canonicalized() {
        let context = Carrier::from_headers(
            [Some(PARENT)].into_iter(),
            [Some("a=1, "), Some("\tb=two words ")].into_iter(),
        );
        assert_eq!(context.parent(), Some(PARENT));
        assert_eq!(context.state(), Some("a=1,b=two words"));
        assert!(
            Carrier::from_headers(
                [Some(PARENT), Some(PARENT)].into_iter(),
                [Some("a=1")].into_iter()
            )
            .parent()
            .is_none()
        );
        assert!(Carrier::from_parts(None, Some("a=1")).state().is_none());
    }

    #[test]
    fn state_errors_do_not_destroy_a_valid_parent() {
        for state in [
            "a=1,a=2", "a=", "=b", "1x=v", "a@2b=v", "a=b\tc", "a=b\rc", "a=b\nc", "a=é", "a=b=c",
        ] {
            let context = Carrier::from_parts(Some(PARENT), Some(state));
            assert_eq!(context.parent(), Some(PARENT));
            assert!(context.state().is_none(), "invalid state accepted");
        }
        let context = Carrier::from_parts(Some(PARENT), Some("1tenant@vendor=opaque,_invalid=x"));
        assert!(context.state().is_none());
        assert_eq!(
            Carrier::from_parts(Some(PARENT), Some("1tenant@vendor=opaque")).state(),
            Some("1tenant@vendor=opaque")
        );
    }

    #[test]
    fn state_and_member_bounds_are_hard_limits() {
        let oversized = [b'a'; STATE_BYTES + 1];
        assert!(State::parse(core::str::from_utf8(&oversized).expect("ASCII")).is_none());
        let empty_members = [b','; STATE_MEMBERS];
        assert!(State::parse(core::str::from_utf8(&empty_members).expect("ASCII")).is_none());
        let parent = Carrier::from_headers(
            [Some(PARENT)].into_iter(),
            [Some("a=1"); STATE_MEMBERS + 1].into_iter(),
        );
        assert_eq!(parent.parent(), Some(PARENT));
        assert!(parent.state().is_none());
    }

    #[test]
    fn actual_span_replaces_parent_and_drops_oversized_or_duplicate_sdk_state() {
        let mut carrier = Carrier::from_parts(Some(PARENT), Some("old=state"));
        carrier.set_span(&[1; 16], &[2; 8], false, [("new", "state")].into_iter());
        assert_eq!(
            carrier.parent(),
            Some("00-01010101010101010101010101010101-0202020202020202-00")
        );
        assert_eq!(carrier.state(), Some("new=state"));
        carrier.set_span(
            &[1; 16],
            &[2; 8],
            true,
            [("a", "1"), ("a", "2")].into_iter(),
        );
        assert!(carrier.state().is_none());
        carrier.set_span(&[0; 16], &[2; 8], false, core::iter::empty());
        assert!(carrier.parent().is_none());
    }
}
