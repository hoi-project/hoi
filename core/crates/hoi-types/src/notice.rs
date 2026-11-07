//! Operator notices (PROTOCOL.md §14.5): what the operator of a public node tells
//! people before the node serves them: what it processes and keeps and, when the operator says,
//! who runs it, how to reach them and where the server is. A node carries its notice in its hello
//! to strangers and, signed by its endpoint key, in its node link, so a client shows it before it
//! says anything of its own, and before it connects at all when it was given the link.
//!
//! Who and where are asked for, never forced: an operator may leave the name, the contact, the
//! country and the provider empty, and the fixed sentences then say plainly that they did.
//! Whatever they fill in is shown to everyone who connects to the node and travels in its link.
//!
//! Pure data, no I/O. The file an operator writes is JSON with camelCase keys (`serde`); what
//! travels is canonical CBOR with one-letter keys, hashed with BLAKE3, so one notice has one hash
//! wherever it is read. The fields are the operator's own words; the fixed sentences people read
//! around them ([`NodeNotice::sections`]) are the software's, the same in every client.

use serde::{Deserialize, Serialize};

use crate::cbor::{self, Value};
use crate::{Id, TypesError};

/// The notice format this version writes and reads.
pub const NOTICE_VERSION: u64 = 1;
/// Canonical bytes a notice may take: it rides in every hello to a stranger and, deflated, in a
/// node link that should still fit a QR code.
pub const MAX_NOTICE_BYTES: usize = 1536;
/// What a hello's `nt` or a link may carry at most, room for a later version's fields included.
pub const MAX_NOTICE_WIRE: usize = 2048;
/// Field caps, in characters.
pub const MAX_NODE_NAME: usize = 64;
pub const MAX_OPERATOR: usize = 120;
pub const MAX_CONTACT: usize = 200;
pub const MAX_PROVIDER: usize = 120;
pub const MAX_PROCESSES: usize = 600;
pub const MAX_PURPOSE: usize = 400;
pub const MAX_RULES: usize = 600;
pub const MAX_EXTRA: usize = 600;
pub const MAX_RETENTION_DAYS: u16 = 3650;
/// The first line of the statement a node's endpoint key signs to bind a notice to its link.
pub const STATEMENT_PREFIX: &str = "hoi node notice v1";

/// The default wording for what a node processes; the operator adds to it or replaces it.
pub const DEFAULT_PROCESSES: &str = "Nothing beyond that: the node keeps the open communities it carries (their posts and media, which are public) and the addresses of the other public nodes it reached.";
/// The default purpose line.
pub const DEFAULT_PURPOSE: &str = "The data is used only to operate this node and the communities it carries. It is not sold or used for advertising.";
/// What every notice says about the other people a device meets.
pub const OTHER_PARTICIPANTS: &str =
    "Members of a community can see your IP address through direct peer-to-peer connections. The operator cannot control what they do with it.";
/// What every notice says about the rest of the network: this node is a way in, not a boundary.
pub const BEYOND_THIS_NODE: &str = "This node is one way into the network, not the only one. When you join a community, open an invite or a link, read, post, reply or sync, your device may also connect to other nodes and participants anywhere on the internet that carry those communities, and which ones changes over time. Public nodes that carry a notice show you theirs before your device tells them anything about you; participants' devices, and nodes without a notice, show none.";

/// What every notice of a node that keeps pieces for others says (SCATTER.md §5.12): the node
/// software sets the flag from how it runs, so the sentence is never missing or wrong.
pub const PIECES: &str = "This node also keeps encrypted pieces of posts and encrypted messages of private communities for other people's devices, and hands them back when asked. It cannot read them. Each is deleted when it expires, after 400 days at most.";

/// One notice, as its operator wrote it. `v` is [`NOTICE_VERSION`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NodeNotice {
    pub v: u64,
    /// The node's label, as its operator calls it.
    pub node: String,
    /// Name or pseudonym of the operator; empty when they keep it to themselves. Asked for, not
    /// required: whatever is here is shown to everyone who connects.
    pub operator: String,
    /// How people reach the operator: an email address, a page, a handle; empty when not given.
    pub contact: String,
    /// Where the server stands: ISO 3166-1 alpha-2, upper case; empty when not given.
    pub country: String,
    /// Who hosts the server, when the operator says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// What the node processes beyond addresses and connection data, in the operator's words (required).
    pub processes: String,
    /// Whether the host keeps logs of IP addresses (the node software itself never does), and for how long.
    pub ip_logging: bool,
    pub retention_days: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_rules: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    /// When the operator last changed it, `YYYY-MM-DD` (required).
    pub updated: String,
    /// The node keeps encrypted pieces for others ([`PIECES`]). Set by the node from how it runs
    /// (`hoi-node --hold-pieces`), whatever the file says; written only when true.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pieces: bool,
}

impl Default for NodeNotice {
    fn default() -> Self {
        NodeNotice {
            v: NOTICE_VERSION,
            node: String::new(),
            operator: String::new(),
            contact: String::new(),
            country: String::new(),
            provider: None,
            processes: String::new(),
            ip_logging: false,
            retention_days: 0,
            purpose: None,
            community_rules: None,
            extra: None,
            updated: String::new(),
            pieces: false,
        }
    }
}

/// Something that keeps a notice from being used, by field: the installers and the app show
/// these words next to the field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// One part of the notice as people read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub text: String,
}

fn problem(field: &str, message: impl Into<String>) -> Problem {
    Problem { field: field.into(), message: message.into() }
}

fn opt(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Letters, digits and ordinary punctuation; newlines only where `multiline`.
fn clean(s: &str, multiline: bool) -> bool {
    s.chars().all(|c| !c.is_control() || (multiline && c == '\n'))
}

impl NodeNotice {
    /// A notice with the fixed defaults filled in, for an operator to complete.
    pub fn template(node: &str) -> NodeNotice {
        NodeNotice { node: node.into(), processes: DEFAULT_PROCESSES.into(), purpose: Some(DEFAULT_PURPOSE.into()), ..NodeNotice::default() }
    }

    /// Everything that keeps this notice from being used, in field order; empty when it is good.
    pub fn validate(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if self.v != NOTICE_VERSION {
            out.push(problem("v", format!("This software writes notice version {NOTICE_VERSION}.")));
        }
        let mut text = |field: &str, value: &str, max: usize, required: bool, multiline: bool, ask: &str| {
            let value = value.trim();
            if value.is_empty() {
                if required {
                    out.push(problem(field, ask));
                }
            } else if value.chars().count() > max {
                out.push(problem(field, format!("At most {max} characters.")));
            } else if !clean(value, multiline) {
                out.push(problem(field, "No control characters."));
            }
        };
        text("node", &self.node, MAX_NODE_NAME, false, false, "");
        text("operator", &self.operator, MAX_OPERATOR, false, false, "");
        text("contact", &self.contact, MAX_CONTACT, false, false, "");
        text("provider", self.provider.as_deref().unwrap_or(""), MAX_PROVIDER, false, false, "");
        text("processes", &self.processes, MAX_PROCESSES, true, true, "Say what this node processes.");
        text("purpose", self.purpose.as_deref().unwrap_or(""), MAX_PURPOSE, false, true, "");
        text("communityRules", self.community_rules.as_deref().unwrap_or(""), MAX_RULES, false, true, "");
        text("extra", self.extra.as_deref().unwrap_or(""), MAX_EXTRA, false, true, "");
        let country = self.country.trim();
        if !country.is_empty() && !(country.len() == 2 && country.bytes().all(|b| b.is_ascii_uppercase())) {
            out.push(problem("country", "Give the server's country as its two-letter ISO code, or leave it empty."));
        }
        if self.ip_logging && self.retention_days == 0 {
            out.push(problem("retentionDays", "Say for how many days IP addresses are kept."));
        } else if self.retention_days > MAX_RETENTION_DAYS {
            out.push(problem("retentionDays", format!("At most {MAX_RETENTION_DAYS} days.")));
        } else if !self.ip_logging && self.retention_days != 0 {
            out.push(problem("retentionDays", "Retention days go with IP logging."));
        }
        if !valid_date(&self.updated) {
            out.push(problem("updated", "Date the notice as YYYY-MM-DD."));
        }
        if out.is_empty() {
            let size = self.canonical().len();
            if size > MAX_NOTICE_BYTES {
                out.push(problem("notice", format!("Too long for a node link: shorten it by {} bytes (at most {MAX_NOTICE_BYTES}).", size - MAX_NOTICE_BYTES)));
            }
        }
        out
    }

    /// The bytes that travel and are hashed: a canonical CBOR map with one-letter keys, trimmed
    /// text, optional fields left out when empty. Operator, contact and country are always
    /// written, empty when not given, so a reader from before they became optional still reads it.
    pub fn canonical(&self) -> Vec<u8> {
        let t = |s: &str| Value::T(s.trim().to_string());
        let mut e = vec![
            ("v", Value::U(self.v)),
            ("n", t(&self.node)),
            ("o", t(&self.operator)),
            ("c", t(&self.contact)),
            ("k", t(&self.country)),
            ("p", t(&self.processes)),
            ("l", Value::Bool(self.ip_logging)),
            ("r", Value::U(u64::from(self.retention_days))),
            ("d", t(&self.updated)),
        ];
        for (key, value) in [("h", &self.provider), ("u", &self.purpose), ("g", &self.community_rules), ("x", &self.extra)] {
            if let Some(v) = opt(value) {
                e.push((key, t(v)));
            }
        }
        if self.pieces {
            e.push(("s", Value::Bool(true)));
        }
        Value::map(e).encode()
    }

    /// Reads what [`NodeNotice::canonical`] wrote. Keys this version doesn't know are ignored, so
    /// a later version's notice still reads; the receiver keeps the bytes it got for the hash.
    /// Only what the node processes must be there: who and where may be empty or absent.
    pub fn from_canonical(bytes: &[u8]) -> Result<NodeNotice, TypesError> {
        if bytes.len() > MAX_NOTICE_WIRE {
            return Err(TypesError::Field("notice too long".into()));
        }
        let Value::M(entries) = cbor::decode(bytes)? else { return Err(TypesError::Field("notice must be a map".into())) };
        let mut n = NodeNotice::default();
        let mut processes = false;
        for (k, v) in entries {
            match (k.as_str(), v) {
                ("v", Value::U(x)) => n.v = x,
                ("n", Value::T(s)) => n.node = s,
                ("o", Value::T(s)) => n.operator = s,
                ("c", Value::T(s)) => n.contact = s,
                ("k", Value::T(s)) => n.country = s,
                ("p", Value::T(s)) => {
                    n.processes = s;
                    processes = true;
                }
                ("l", Value::Bool(b)) => n.ip_logging = b,
                ("r", Value::U(x)) => n.retention_days = u16::try_from(x).map_err(|_| TypesError::Field("retention out of range".into()))?,
                ("d", Value::T(s)) => n.updated = s,
                ("h", Value::T(s)) => n.provider = Some(s),
                ("u", Value::T(s)) => n.purpose = Some(s),
                ("g", Value::T(s)) => n.community_rules = Some(s),
                ("x", Value::T(s)) => n.extra = Some(s),
                ("s", Value::Bool(b)) => n.pieces = b,
                ("v" | "n" | "o" | "c" | "k" | "p" | "l" | "r" | "d" | "h" | "u" | "g" | "x" | "s", _) => {
                    return Err(TypesError::Field(format!("notice field {k} has the wrong type")));
                }
                _ => {}
            }
        }
        if !processes {
            return Err(TypesError::Field("notice lacks a required field".into()));
        }
        if n.v > NOTICE_VERSION {
            // Readable, the fields this version knows at least; the hash still covers it all.
            n.v = NOTICE_VERSION;
        }
        Ok(n)
    }

    /// `BLAKE3(canonical)`: what identifies this exact wording.
    pub fn hash(&self) -> Id {
        hash_of(&self.canonical())
    }

    /// The notice as people read it: the operator's words inside the fixed sentences every
    /// client shows, in this order. Where the operator left who or where out, the sentences say
    /// so. The footer (version and date) is [`NodeNotice::footer`].
    pub fn sections(&self) -> Vec<Section> {
        let s = |title: &str, text: String| Section { title: title.into(), text };
        let operator = self.operator.trim();
        let contact = self.contact.trim();
        let country = self.country.trim();
        let mut out = vec![
            s(
                "Operator",
                match (operator, contact) {
                    ("", "") => "The operator of this node chose not to say who they are or how to reach them.".into(),
                    ("", c) => format!("The operator of this node gave no name. Contact: {c}."),
                    (o, "") => format!("This node is operated by {o}, who gave no contact."),
                    (o, c) => format!("This node is operated by {o}. Contact: {c}."),
                },
            ),
            s(
                "Server location",
                match (country, opt(&self.provider)) {
                    ("", None) => "The operator did not say where the server is located or who hosts it.".into(),
                    ("", Some(p)) => format!("The server is hosted by {p}. The operator did not say in which country."),
                    (k, None) => format!("The server is located in {k}."),
                    (k, Some(p)) => format!("The server is located in {k} and hosted by {p}."),
                },
            ),
            s(
                "What this node processes",
                format!(
                    "To connect you with communities, this node processes your IP address, connection times and technical connection data. {}",
                    self.processes.trim()
                ),
            ),
            s(
                "Logging and retention",
                if self.ip_logging {
                    format!(
                        "IP addresses are logged for {} day{} for security and abuse prevention, then deleted.",
                        self.retention_days,
                        if self.retention_days == 1 { "" } else { "s" }
                    )
                } else {
                    "IP addresses are not logged.".into()
                },
            ),
        ];
        if self.pieces {
            // right after what the node processes: it is part of what the node keeps
            out.insert(3, s("Encrypted pieces", PIECES.into()));
        }
        out.extend([
            s("Purpose", opt(&self.purpose).unwrap_or(DEFAULT_PURPOSE).into()),
            s("Other participants", OTHER_PARTICIPANTS.into()),
            s("Beyond this node", BEYOND_THIS_NODE.into()),
        ]);
        if let Some(rules) = opt(&self.community_rules) {
            out.push(s("Community rules", rules.into()));
        }
        if let Some(extra) = opt(&self.extra) {
            out.push(s("More", extra.into()));
        }
        out.push(s(
            "Your rights",
            if contact.is_empty() {
                "The operator gave no contact, so this notice offers no way to ask them what data about you this node holds or to request its deletion. If that matters to you, decline and use another node.".into()
            } else {
                "You can ask the operator what data about you this node holds and request its deletion via the contact above.".into()
            },
        ));
        out
    }

    /// Who wrote this notice, in a few words for a terminal: `by Lena · contact lena@example.org`.
    pub fn byline(&self) -> String {
        let operator = match self.operator.trim() {
            "" => "by an operator who gave no name".to_string(),
            o => format!("by {o}"),
        };
        let contact = match self.contact.trim() {
            "" => "no contact given".to_string(),
            c => format!("contact {c}"),
        };
        match self.country.trim() {
            "" => format!("{operator} · {contact}"),
            k => format!("{operator} · {contact} · server in {k}"),
        }
    }

    /// What an operator who left who or where out should know: people are told so.
    pub fn omissions(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.operator.trim().is_empty() {
            out.push("no operator name: people who connect read that you chose not to say who you are");
        }
        if self.contact.trim().is_empty() {
            out.push("no contact: people who connect read that they cannot reach you about their data");
        }
        if self.country.trim().is_empty() {
            out.push("no country: people who connect read that you did not say where the server is");
        }
        out
    }

    /// `Join notice – NAME`, or `Join notice` for a node without a label.
    pub fn title(&self) -> String {
        match self.node.trim() {
            "" => "Join notice".into(),
            name => format!("Join notice – {name}"),
        }
    }

    /// `Notice version 1a2b3c4d5e6f, last updated 2026-11-03.`
    pub fn footer(&self) -> String {
        format!("Notice version {}, last updated {}.", &self.hash().hex()[..12], self.updated.trim())
    }

    /// The whole notice as plain text, for a terminal or a log.
    pub fn text(&self) -> String {
        let mut out = self.title();
        out.push('\n');
        for s in self.sections() {
            out.push_str(&format!("\n{}. {}\n", s.title, s.text));
        }
        out.push('\n');
        out.push_str(&self.footer());
        out.push('\n');
        out
    }
}

/// `BLAKE3` of the bytes a notice travels as.
pub fn hash_of(canonical: &[u8]) -> Id {
    Id(*blake3::hash(canonical).as_bytes())
}

/// The exact UTF-8 message a node's endpoint key signs for its link: the key, so a notice signed
/// for one node means nothing in another's link, and the notice's hash; no trailing newline.
pub fn statement(endpoint: &Id, hash: &Id) -> String {
    format!("{STATEMENT_PREFIX}\n{}\n{}", endpoint.hex(), hash.hex())
}

/// `YYYY-MM-DD`, with a plausible month and day; no calendar beyond that.
pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().ok();
    match (num(0..4), num(5..7), num(8..10)) {
        (Some(y), Some(m), Some(d)) => (2024..=2999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> NodeNotice {
        NodeNotice {
            node: "sleepy-walrus-48213".into(),
            operator: "Lena".into(),
            contact: "lena@example.org".into(),
            country: "XX".into(),
            provider: Some("Example Hosting".into()),
            ip_logging: true,
            retention_days: 7,
            updated: "2026-11-03".into(),
            ..NodeNotice::template("sleepy-walrus-48213")
        }
    }

    #[test]
    fn a_complete_notice_passes_and_round_trips() {
        let n = good();
        assert!(n.validate().is_empty(), "{:?}", n.validate());
        let bytes = n.canonical();
        assert!(bytes.len() < MAX_NOTICE_BYTES);
        let back = NodeNotice::from_canonical(&bytes).unwrap();
        assert_eq!(back, n);
        assert_eq!(back.hash(), n.hash());
        assert_eq!(hash_of(&bytes), n.hash());
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.contains("\"ipLogging\":true") && json.contains("\"retentionDays\":7") && !json.contains("extra"));
        assert_eq!(serde_json::from_str::<NodeNotice>(&json).unwrap(), n);
    }

    #[test]
    fn trimming_and_absent_optionals_give_the_same_bytes() {
        let mut a = good();
        a.operator = "  Lena ".into();
        a.extra = Some("   ".into());
        assert_eq!(a.canonical(), good().canonical(), "whitespace around a field and an empty optional change nothing");
        let mut b = good();
        b.provider = None;
        assert_ne!(b.canonical(), good().canonical());
    }

    #[test]
    fn what_it_processes_and_the_date_are_required_who_and_where_are_not() {
        let empty = NodeNotice::default();
        let fields: Vec<String> = empty.validate().into_iter().map(|p| p.field).collect();
        assert_eq!(fields, ["processes", "updated"], "who and where may be left out");
        let mut anonymous = good();
        anonymous.operator = String::new();
        anonymous.contact = "  ".into();
        anonymous.country = String::new();
        anonymous.provider = None;
        assert!(anonymous.validate().is_empty(), "{:?}", anonymous.validate());
        let back = NodeNotice::from_canonical(&anonymous.canonical()).unwrap();
        assert_eq!((back.operator.as_str(), back.contact.as_str(), back.country.as_str()), ("", "", ""));
        let mut n = good();
        n.ip_logging = true;
        n.retention_days = 0;
        assert_eq!(n.validate()[0].field, "retentionDays");
        n.ip_logging = false;
        n.retention_days = 3;
        assert_eq!(n.validate()[0].field, "retentionDays");
        let mut n = good();
        n.country = "xx".into();
        assert_eq!(n.validate()[0].field, "country");
        n.country = "Somewhere".into();
        assert_eq!(n.validate()[0].message, "Give the server's country as its two-letter ISO code, or leave it empty.");
        n.country = "XX".into();
        n.updated = "3.11.2026".into();
        assert_eq!(n.validate()[0].field, "updated");
        n.updated = "2026-11-03".into();
        n.operator = "x".repeat(MAX_OPERATOR + 1);
        assert_eq!(n.validate()[0].message, format!("At most {MAX_OPERATOR} characters."));
        n.operator = "Le\u{7}na".into();
        assert_eq!(n.validate()[0].message, "No control characters.");
        n.operator = "Lena".into();
        n.processes = "a line\nand another".into();
        assert!(n.validate().is_empty(), "newlines are fine in a paragraph");
        n.v = 2;
        assert_eq!(n.validate()[0].field, "v");
    }

    #[test]
    fn the_budget_keeps_a_link_scannable() {
        let mut n = good();
        n.processes = "x".repeat(MAX_PROCESSES);
        n.purpose = Some("y".repeat(MAX_PURPOSE));
        n.community_rules = Some("z".repeat(MAX_RULES));
        let problems = n.validate();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].field, "notice");
        assert!(problems[0].message.starts_with("Too long for a node link"));
        assert!(NodeNotice::from_canonical(&vec![0; MAX_NOTICE_WIRE + 1]).is_err());
    }

    #[test]
    fn the_text_people_read_carries_the_operators_words_in_fixed_sentences() {
        let n = good();
        let text = n.text();
        assert!(text.starts_with("Join notice – sleepy-walrus-48213\n"));
        assert!(text.contains("Operator. This node is operated by Lena. Contact: lena@example.org."));
        assert!(text.contains("Server location. The server is located in XX and hosted by Example Hosting."));
        assert!(text.contains(&format!("Beyond this node. {BEYOND_THIS_NODE}")));
        assert!(text.contains("via the contact above."));
        assert!(text.contains("IP addresses are logged for 7 days for security and abuse prevention, then deleted."));
        assert!(text.contains("Your rights."));
        assert!(text.ends_with(&format!("{}\n", n.footer())));
        let mut quiet = good();
        quiet.ip_logging = false;
        quiet.retention_days = 0;
        quiet.community_rules = Some("Be kind.".into());
        let sections = quiet.sections();
        assert!(sections.iter().any(|s| s.title == "Logging and retention" && s.text == "IP addresses are not logged."));
        assert!(sections.iter().any(|s| s.title == "Community rules" && s.text == "Be kind."));
        assert_eq!(good().byline(), "by Lena · contact lena@example.org · server in XX");
        assert!(good().omissions().is_empty());
    }

    #[test]
    fn what_an_operator_leaves_out_is_said_to_everyone_who_connects() {
        let text_of = |n: &NodeNotice, title: &str| n.sections().into_iter().find(|s| s.title == title).map(|s| s.text).unwrap();
        let mut n = good();
        n.contact = String::new();
        assert_eq!(text_of(&n, "Operator"), "This node is operated by Lena, who gave no contact.");
        assert!(text_of(&n, "Your rights").starts_with("The operator gave no contact, so this notice offers no way to ask them"));
        n.operator = String::new();
        assert_eq!(text_of(&n, "Operator"), "The operator of this node chose not to say who they are or how to reach them.");
        n.contact = "lena@example.org".into();
        assert_eq!(text_of(&n, "Operator"), "The operator of this node gave no name. Contact: lena@example.org.");
        n.country = String::new();
        assert_eq!(text_of(&n, "Server location"), "The server is hosted by Example Hosting. The operator did not say in which country.");
        n.provider = None;
        assert_eq!(text_of(&n, "Server location"), "The operator did not say where the server is located or who hosts it.");
        n.contact = String::new();
        assert_eq!(n.byline(), "by an operator who gave no name · no contact given");
        assert_eq!(n.omissions().len(), 3);
    }

    #[test]
    fn a_later_version_still_reads_and_a_wrong_type_does_not() {
        let mut e: Vec<(&str, Value)> = vec![
            ("v", Value::U(2)),
            ("n", Value::T("n".into())),
            ("o", Value::T("o".into())),
            ("c", Value::T("c".into())),
            ("k", Value::T("XX".into())),
            ("p", Value::T("p".into())),
            ("l", Value::Bool(false)),
            ("r", Value::U(0)),
            ("d", Value::T("2026-11-03".into())),
            ("zz", Value::T("from the future".into())),
        ];
        let n = NodeNotice::from_canonical(&Value::map(e.clone()).encode()).unwrap();
        assert_eq!((n.v, n.operator.as_str()), (NOTICE_VERSION, "o"));
        let without_who: Vec<(&str, Value)> = e.iter().filter(|(k, _)| !["o", "c", "k"].contains(k)).cloned().collect();
        assert_eq!(NodeNotice::from_canonical(&Value::map(without_who).encode()).unwrap().operator, "", "who and where may be absent");
        let without_processes: Vec<(&str, Value)> = e.iter().filter(|(k, _)| *k != "p").cloned().collect();
        assert!(NodeNotice::from_canonical(&Value::map(without_processes).encode()).is_err());
        e[1] = ("n", Value::U(5));
        assert!(NodeNotice::from_canonical(&Value::map(e).encode()).is_err());
        assert_eq!(statement(&Id([1; 32]), &Id([2; 32])), format!("hoi node notice v1\n{}\n{}", Id([1; 32]).hex(), Id([2; 32]).hex()));
        assert!(valid_date("2026-02-30") && !valid_date("2026-13-01") && !valid_date("26-1-1"));
    }

    #[test]
    fn a_node_that_keeps_pieces_says_so_in_a_fixed_sentence() {
        let plain = good();
        let mut n = good();
        n.pieces = true;
        let bytes = n.canonical();
        assert_ne!(bytes, plain.canonical(), "a different notice, a different hash: people see it again");
        assert_eq!(NodeNotice::from_canonical(&bytes).unwrap(), n);
        let sections = n.sections();
        let at = sections.iter().position(|s| s.title == "Encrypted pieces").expect("the sentence is there");
        assert_eq!(sections[at - 1].title, "What this node processes");
        assert_eq!(sections[at].text, PIECES);
        assert!(!plain.sections().iter().any(|s| s.title == "Encrypted pieces"));
        // in the operator's file it is written only when true, and older files read as false
        assert!(!serde_json::to_string(&plain).unwrap().contains("pieces"));
        assert!(serde_json::to_string(&n).unwrap().contains("\"pieces\":true"));
        // a wrong type is refused like any other field's
        let mut e: Vec<(&str, Value)> = vec![("p", Value::T("p".into())), ("s", Value::T("yes".into()))];
        assert!(NodeNotice::from_canonical(&Value::map(e.clone()).encode()).is_err());
        e[1] = ("s", Value::Bool(true));
        assert!(NodeNotice::from_canonical(&Value::map(e).encode()).unwrap().pieces);
    }
}
