//! `hoi-sim`: pentest the hoi protocol and watch it hold.
//!
//!   hoi-sim battle [--honest N] [--attackers M] [--days D] [--pow-bits B] [--json]
//!       One process, accelerated time, honest vs adversarial actors, a scored verdict. No network.
//!
//!   hoi-sim agent  --data DIR [--hub URL] [--name NAME] <verb> [args]
//!       One participant against a running hub (a hoi-node). Persists its identity in DIR, syncs on
//!       every call, does one thing, reports JSON. Run it as many times, from as many sessions, as
//!       you like — each DIR is an independent member. Honest verbs and forging live here.
//!
//!   hoi-sim ladder [--days D] [--seed S]
//!       The trust ladder and consent with hundreds of members against Sybil rings, a turncoat
//!       and patient Sybils, settled with the core's own ladder.
//!
//!   hoi-sim referee --data DIR [--hub URL] [--community HEX] [--watch] [--json]
//!       An honest observer. Syncs and prints a scoreboard of community health: who stands where,
//!       whose voice is what, what is napping, who is banned, which proposals are pending.
//!
//! The container the agent runs in is the real boundary; this binary just speaks the protocol.

use std::path::PathBuf;
use std::process::ExitCode;

use hoi_core::{Node, Source};
use hoi_sim::forge::{self, Expect};
use hoi_sim::{WEB_BASE, scenario, wire};
use serde_json::{Value, json};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    match cmd {
        "battle" => battle(rest),
        "agent" => run_async(|| agent(rest)),
        "referee" => run_async(|| referee(rest)),
        "scatter" => scatter(rest),
        "ladder" => ladder(rest),
        "-h" | "--help" | "help" | "" => {
            print_help();
            ExitCode::SUCCESS
        }
        "-V" | "--version" => {
            println!("hoi-sim {}", hoi_core::VERSION);
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("hoi-sim: unknown command {other:?}\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

/// `hoi-sim scatter [--trials N] [--seed S] [--life [--objects N] [--days D]] [--load]
/// [--address]`: the placement report, objects over their whole life with churn, repairs, forced
/// repairs and bad updates, a peak hour at 10,000 to 200,000 online, or who can link a device's
/// address to its records (SCATTER.md §10, §13, §15).
fn scatter(args: &[String]) -> ExitCode {
    let (mut trials, mut seed) = (20_000u64, 1u64);
    let (mut life, mut objects, mut days, mut load, mut address) = (false, 1000u32, 90u32, false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--trials" => trials = it.next().and_then(|s| s.parse().ok()).unwrap_or(trials),
            "--seed" => seed = it.next().and_then(|s| s.parse().ok()).unwrap_or(seed),
            "--life" => life = true,
            "--load" => load = true,
            "--address" => address = true,
            "--objects" => objects = it.next().and_then(|s| s.parse().ok()).unwrap_or(objects),
            "--days" => days = it.next().and_then(|s| s.parse().ok()).unwrap_or(days),
            other => {
                eprintln!("hoi-sim scatter: unknown option {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    let lines = if address {
        hoi_sim::address::report(trials.min(u32::MAX as u64) as u32, seed)
    } else if load {
        hoi_sim::load::report(seed)
    } else if life {
        hoi_sim::life::report(objects, days, seed)
    } else {
        hoi_sim::scatter::report(trials, seed)
    };
    for line in lines {
        println!("{line}");
    }
    ExitCode::SUCCESS
}

/// `hoi-sim ladder [--days D] [--seed S]`: the ladder and consent against adversarial vouching.
fn ladder(args: &[String]) -> ExitCode {
    let (mut days, mut seed) = (360u32, 1u64);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--days" => days = it.next().and_then(|s| s.parse().ok()).unwrap_or(days),
            "--seed" => seed = it.next().and_then(|s| s.parse().ok()).unwrap_or(seed),
            other => {
                eprintln!("hoi-sim ladder: unknown option {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }
    for line in hoi_sim::ladder::report(seed, days) {
        println!("{line}");
    }
    ExitCode::SUCCESS
}

fn print_help() {
    eprintln!(
        "hoi-sim {} — pentest the hoi protocol and watch it hold\n\n\
         commands:\n\
         \x20 battle    in-process honest-vs-adversarial siege with a scored verdict\n\
         \x20 agent     one participant against a running hub (honest actions and forging)\n\
         \x20 referee   an honest observer's scoreboard of community health\n\
         \x20 scatter   place pieces with the core's rules against a party's share of places\n\
         \x20 ladder    the trust ladder and consent against adversarial vouching\n\n\
         try:  hoi-sim battle --honest 4 --attackers 3\n\
         \x20     hoi-sim agent --data ./m --hub ws://hub:7842 --name mallory init\n\
         \x20     hoi-sim agent --data ./m forge bad-sig <community-hex>\n\
         \x20     hoi-sim referee --data ./ref --hub ws://hub:7842 --watch\n\n\
         forge attacks: {}",
        hoi_core::VERSION,
        forge::catalogue().join(", ")
    );
}

// A tiny current-thread runtime, so the node (not Send across awaits) stays on one thread.
fn run_async(f: impl FnOnce() -> ExitCode) -> ExitCode {
    f()
}

// ---------------------------------------------------------------------------- flag parsing

struct Flags {
    data: Option<PathBuf>,
    hub: Option<String>,
    name: Option<String>,
    community: Option<String>,
    watch: bool,
    json: bool,
    honest: usize,
    attackers: usize,
    days: u64,
    pow_bits: Option<u32>,
    positional: Vec<String>,
}

fn parse(rest: &[String]) -> Flags {
    let mut f = Flags {
        data: None,
        hub: None,
        name: None,
        community: None,
        watch: false,
        json: false,
        honest: 4,
        attackers: 3,
        days: 30,
        pow_bits: None,
        positional: Vec::new(),
    };
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data" => f.data = it.next().map(PathBuf::from),
            "--hub" => f.hub = it.next().cloned(),
            "--name" => f.name = it.next().cloned(),
            "--community" | "-c" => f.community = it.next().cloned(),
            "--watch" => f.watch = true,
            "--json" => f.json = true,
            "--honest" => f.honest = it.next().and_then(|s| s.parse().ok()).unwrap_or(f.honest),
            "--attackers" => f.attackers = it.next().and_then(|s| s.parse().ok()).unwrap_or(f.attackers),
            "--days" => f.days = it.next().and_then(|s| s.parse().ok()).unwrap_or(f.days),
            "--pow-bits" => f.pow_bits = it.next().and_then(|s| s.parse().ok()),
            other => f.positional.push(other.to_string()),
        }
    }
    f
}

// ---------------------------------------------------------------------------- battle

fn battle(rest: &[String]) -> ExitCode {
    let f = parse(rest);
    let card = scenario::siege(f.honest, f.attackers, f.days, f.pow_bits);
    if f.json {
        println!("{}", serde_json::to_string_pretty(&card).unwrap());
    } else {
        println!("{}", card.render());
    }
    if card.held() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

// ---------------------------------------------------------------------------- node open

fn open(f: &Flags, platform: &str) -> Result<Node, String> {
    let dir = f.data.clone().ok_or("this command needs --data DIR")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
    let db = hoi_store::sqlite::Sqlite::open(&dir.join("hoi.db")).map_err(|e| format!("storage: {e}"))?;
    let mut node = Node::open(Box::new(db), platform, WEB_BASE, hoi_core::system_clock()).map_err(|e| e.to_string())?;
    node.set_identity_bits(hoi_crypto::identity::DEFAULT_BITS);
    Ok(node)
}

fn hub_url(f: &Flags, node: &Node) -> Option<String> {
    f.hub.clone().or_else(|| node.home().map(|h| h.url))
}

fn out(json: Value) -> ExitCode {
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
    ExitCode::SUCCESS
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    println!("{}", serde_json::to_string_pretty(&json!({ "ok": false, "error": msg.to_string() })).unwrap());
    ExitCode::FAILURE
}

// A minimal current-thread tokio runtime for one network session.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime").block_on(fut)
}

// ---------------------------------------------------------------------------- agent

fn agent(rest: &[String]) -> ExitCode {
    let f = parse(rest);
    let verb = f.positional.first().cloned().unwrap_or_default();
    let a = &f.positional[f.positional.len().min(1)..];

    let mut node = match open(&f, "desktop") {
        Ok(n) => n,
        Err(e) => return fail(e),
    };

    match verb.as_str() {
        // Local-only setup verbs: no hub needed.
        "init" => {
            if node.me().is_some() {
                let me = node.me().unwrap();
                return out(json!({ "ok": true, "already": true, "key": me.root, "short": me.short }));
            }
            let name = f.name.clone().unwrap_or_else(|| "agent".into());
            match node.create_identity(&name) {
                Ok(me) => {
                    node.commit().ok();
                    out(json!({ "ok": true, "key": me.root, "short": me.short, "name": me.name }))
                }
                Err(e) => fail(e.message),
            }
        }
        "pair" => {
            let Some(link) = a.first() else { return fail("usage: agent --data D pair <pair-link>") };
            match node.pair(link) {
                Ok(_) => {
                    node.commit().ok();
                    // Confirm by syncing, if we can.
                    if let Some(url) = hub_url(&f, &node) {
                        let _ = block_on(wire::session(&mut node, &url, |_| ()));
                    }
                    out(json!({ "ok": true, "paired": true, "hub": node.home().map(|h| h.url) }))
                }
                Err(e) => fail(e.message),
            }
        }
        "join" => {
            let Some(invite) = a.first() else { return fail("usage: agent --data D join <invite-uri>") };
            match node.join(invite) {
                Ok(cv) => {
                    node.commit().ok();
                    if let Some(url) = hub_url(&f, &node) {
                        let _ = block_on(wire::session(&mut node, &url, |_| ()));
                    }
                    out(json!({ "ok": true, "joined": cv.id, "name": cv.name }))
                }
                Err(e) => fail(e.message),
            }
        }
        // Found a community (the sandbox needs someone to be the founder). Local, then sync.
        "create" => {
            use hoi_core::dto::{CommunityDraft, LadderPatch, LimitsPatch};
            if node.me().is_none() {
                return fail("no identity yet; run `init --name NAME` first");
            }
            let name = a.first().cloned().unwrap_or_else(|| "sandbox".into());
            // Sandbox-friendly limits: real stamps but quick, and "established" in minutes so a
            // pentest doesn't wait two weeks. Override established minutes with --days N (as minutes).
            let est_secs = if f.days > 0 { f.days * 60 } else { 300 };
            let draft = CommunityDraft {
                name: name.clone(),
                description: "A sandbox community for pentesting the hoi protocol.".into(),
                color: "mint".into(),
                limits: Some(LimitsPatch {
                    pow_bits: f.pow_bits.or(Some(12)),
                    max_retention: Some(7 * 86_400),
                    newcomer_posts_per_day: Some(4),
                    ladder: Some(LadderPatch {
                        identity_bits: Some(8),
                        admit_vouches: Some(1),
                        established_age: Some(est_secs),
                        contest_window: Some(600),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                rules: Some("1. Be kind. 2. No spam. 3. Stay on topic.".into()),
                co_owners: None,
                threshold: None,
            };
            match node.create_community(draft) {
                Ok(cv) => {
                    node.commit().ok();
                    let invite = node.invite(&cv.id).ok();
                    if let Some(url) = hub_url(&f, &node) {
                        let _ = block_on(wire::session(&mut node, &url, |_| ()));
                    }
                    out(json!({ "ok": true, "community": cv.id, "name": cv.name, "invite": invite.map(|i| i.uri) }))
                }
                Err(e) => fail(e.message),
            }
        }
        "invite" => {
            let Some(c) = a.first() else { return fail("usage: agent --data D invite <community-hex>") };
            match node.invite(c) {
                Ok(i) => out(json!({ "ok": true, "invite": i.uri, "web": i.web })),
                Err(e) => fail(e.message),
            }
        }
        // The sandbox owner tab is a real delegated device, approved by the
        // founder's root key. Invites alone never grant owner capabilities.
        "link-approve" => {
            let Some(request) = a.first() else { return fail("usage: link-approve <device-link-request>") };
            match node.link_approve(request) {
                Ok(approval) => {
                    if let Err(e) = node.commit() {
                        return fail(e);
                    }
                    out(json!({ "ok": true, "approval": approval }))
                }
                Err(e) => fail(e.message),
            }
        }
        // Everything else syncs against the hub first.
        "whoami" | "sync" | "feed" | "context" | "thread" | "post" | "post-link" | "comment" | "vote" | "vouch" | "review" | "standing" | "community"
        | "ledger" | "propose-rules" | "propose-limits" | "forge" => {
            if node.me().is_none() && verb != "forge" {
                return fail("no identity yet; run `init --name NAME` first");
            }
            let Some(url) = hub_url(&f, &node) else {
                return fail("no hub; pass --hub ws://… or run `pair <link>` first");
            };
            if verb == "forge" {
                return do_forge(&mut node, &f, a, &url);
            }
            let aa: Vec<String> = a.to_vec();
            let verb2 = verb.clone();
            let result = block_on(wire::session(&mut node, &url, move |n| act(n, &verb2, &aa)));
            match result {
                Ok(v) => out(v),
                Err(e) => fail(e),
            }
        }
        other => fail(format!(
            "unknown verb {other:?}; try: whoami sync feed context thread post post-link comment vote vouch review standing community ledger propose-rules propose-limits forge link-approve"
        )),
    }
}

/// The node mutation/read a verb performs, run after the initial sync.
fn act(n: &mut Node, verb: &str, a: &[String]) -> Value {
    use hoi_core::dto::PostDraft;
    let r = |v: Result<Value, hoi_core::ApiError>| v.unwrap_or_else(|e| json!({ "ok": false, "error": e.message }));
    match verb {
        "whoami" => json!({
            "ok": true,
            "me": n.me(),
            "communities": n.communities().into_iter().map(|c| json!({ "id": c.id, "name": c.name, "joined": c.joined })).collect::<Vec<_>>(),
            "net": serde_json::to_value(n.net_status()).ok(),
        }),
        "sync" => json!({ "ok": true, "communities": n.communities().len(), "net": serde_json::to_value(n.net_status()).ok() }),
        "feed" => {
            let c = a.first().map(String::as_str);
            r(n.feed(c, "fresh", None).map(|posts| {
                json!({
                    "ok": true,
                    "count": posts.len(),
                    "posts": posts.iter().map(|p| json!({
                        "id": p.id, "title": p.title, "author": p.author.key, "napping": p.napping.is_some(),
                        "text": p.excerpt, "link": p.link, "authorName": p.author.name,
                        "score": p.score, "myVote": p.my_vote, "replies": p.replies, "createdAt": p.created_at,
                    })).collect::<Vec<_>>(),
                })
            }))
        }
        "context" => {
            let Some(c) = a.first() else { return json!({ "ok": false, "error": "usage: context <community-hex>" }) };
            r(agent_context(n, c))
        }
        "thread" => {
            let Some(id) = a.first() else { return json!({ "ok": false, "error": "usage: thread <post-id>" }) };
            r(n.post(id).map(|p| json!({ "ok": true, "post": p })))
        }
        "post" => {
            if a.len() < 3 {
                return json!({ "ok": false, "error": "usage: post <community-hex> <title> <text>" });
            }
            r(n.create_post(&PostDraft { community: a[0].clone(), title: a[1].clone(), text: a[2..].join(" "), link: None, media: None })
                .map(|p| json!({ "ok": true, "id": p.id, "title": p.title, "napping": p.napping.is_some() })))
        }
        "post-link" => {
            if a.len() < 3 {
                return json!({ "ok": false, "error": "usage: post-link <community-hex> <title> <url> [text]" });
            }
            r(n.create_post(&PostDraft { community: a[0].clone(), title: a[1].clone(), text: a[3..].join(" "), link: Some(a[2].clone()), media: None })
                .map(|p| json!({ "ok": true, "id": p.id, "title": p.title, "napping": p.napping.is_some() })))
        }
        "comment" => {
            if a.len() < 2 {
                return json!({ "ok": false, "error": "usage: comment <post-or-comment-id> <text>" });
            }
            r(n.reply(&a[0], &a[1..].join(" ")).map(|c| json!({ "ok": true, "id": c.id })))
        }
        "vote" => {
            if a.len() < 2 {
                return json!({ "ok": false, "error": "usage: vote <target-id> <1|-1|0>" });
            }
            let v: i64 = a[1].parse().unwrap_or(0);
            r(n.vote(&a[0], v).map(|tally| json!({ "ok": true, "tally": tally })))
        }
        "vouch" => {
            if a.len() < 2 {
                return json!({ "ok": false, "error": "usage: vouch <community-hex> <key> [1|-1|0] [reason]" });
            }
            let value: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            let reason = a.get(3).map(String::as_str);
            r(n.vouch(&a[0], &a[1], value, reason).map(|s| json!({ "ok": true, "level": s.level, "voice": s.voice })))
        }
        "review" => {
            if a.len() < 2 {
                return json!({ "ok": false, "error": "usage: review <target-id> <ok|hide|veto|none> [reason]" });
            }
            r(n.review(&a[0], &a[1], a.get(2).map(String::as_str)).map(|_| json!({ "ok": true })))
        }
        "standing" => {
            if a.is_empty() {
                return json!({ "ok": false, "error": "usage: standing <community-hex> [key]" });
            }
            r(n.standing(&a[0], a.get(1).map(String::as_str)).map(|s| serde_json::to_value(s).unwrap_or(Value::Null)))
        }
        "community" => {
            if a.is_empty() {
                return json!({ "ok": false, "error": "usage: community <community-hex>" });
            }
            r(n.community(&a[0]).map(|c| serde_json::to_value(c).unwrap_or(Value::Null)))
        }
        "ledger" => {
            let Some(c) = a.first() else { return json!({ "ok": false, "error": "usage: ledger <community-hex>" }) };
            r(n.ledger(c).map(|entries| json!({ "ok": true, "entries": entries })))
        }
        "propose-rules" => {
            if a.len() < 2 {
                return json!({ "ok": false, "error": "usage: propose-rules <community-hex> <text>" });
            }
            r(n.set_rules(&a[0], &a[1..].join(" ")).map(|result| serde_json::to_value(result).unwrap_or(Value::Null)))
        }
        "propose-limits" => {
            if a.len() < 3 {
                return json!({ "ok": false, "error": "usage: propose-limits <community-hex> <field> <value>  (field: identity-bits|pow-bits|newcomer-posts|hide-quorum|…)" });
            }
            match n.community(&a[0]) {
                Ok(cv) => {
                    let mut limits = cv.limits.clone();
                    let val: u64 = a[2].parse().unwrap_or(0);
                    match a[1].as_str() {
                        "identity-bits" => limits.ladder.identity_bits = val as u32,
                        "pow-bits" => limits.pow_bits = val as u32,
                        "newcomer-posts" => limits.newcomer_posts_per_day = val as u32,
                        "hide-quorum" => limits.ladder.hide_quorum = val as u32,
                        "block-quorum" => limits.ladder.block_quorum = val as u32,
                        "veto-quorum" => limits.ladder.veto_quorum = val as u32,
                        "max-retention" => limits.max_retention = val,
                        other => return json!({ "ok": false, "error": format!("unknown field {other:?}") }),
                    }
                    r(n.set_limits(&a[0], &limits).map(|res| serde_json::to_value(res).unwrap_or(Value::Null)))
                }
                Err(e) => json!({ "ok": false, "error": e.message }),
            }
        }
        _ => json!({ "ok": false, "error": "unreachable" }),
    }
}

const CONTEXT_POSTS: usize = 12;
const CONTEXT_COMMENTS: usize = 12;
const CONTEXT_MEMBERS: usize = 24;
const CONTEXT_LEDGER: usize = 12;

fn clipped(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut value: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        value.push('…');
    }
    value
}

/// The same community a person sees, with enough recent conversation to respond to it. All
/// standing and governance fields come from the core; this never assigns simulated votes or
/// privileges. Opening posts also records ordinary reads for the node's auto-vouch behavior.
fn agent_context(n: &mut Node, community: &str) -> Result<Value, hoi_core::ApiError> {
    let cv = n.community(community)?;
    let me = n.standing(community, None)?;
    let mut keys = vec![me.person.key.clone()];
    let mut posts = Vec::new();
    for summary in n.feed(Some(community), "fresh", Some(CONTEXT_POSTS))? {
        let post = n.post(&summary.id)?;
        keys.push(post.summary.author.key.clone());
        let mut comments = Vec::new();
        let mut pending: Vec<_> = post.comments.iter().collect();
        while let Some(comment) = pending.pop() {
            comments.push(comment);
            pending.extend(comment.children.iter());
        }
        // Recent replies matter most, including replies nested under an older comment.
        comments.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        let omitted_comments = comments.len().saturating_sub(CONTEXT_COMMENTS);
        comments.truncate(CONTEXT_COMMENTS);
        comments.reverse();
        let comments: Vec<Value> = comments
            .into_iter()
            .map(|c| {
                keys.push(c.author.key.clone());
                json!({
                    "id": c.id, "parent": c.parent, "author": c.author.key, "authorName": c.author.name,
                    "text": clipped(&c.text, 1200), "score": c.score, "myVote": c.my_vote,
                    "createdAt": c.created_at, "napping": c.napping.is_some(), "removed": c.removed.is_some(),
                })
            })
            .collect();
        posts.push(json!({
            "id": post.summary.id, "title": post.summary.title, "author": post.summary.author.key,
            "authorName": post.summary.author.name, "text": clipped(&post.text, 4000), "link": post.summary.link,
            "score": post.summary.score, "myVote": post.summary.my_vote, "replies": post.summary.replies,
            "createdAt": post.summary.created_at, "napping": post.summary.napping.is_some(),
            "comments": comments, "omittedComments": omitted_comments,
        }));
    }
    keys.extend(cv.owners.iter().chain(&cv.mods).chain(&cv.pending).map(|p| p.key.clone()));
    let mut seen = std::collections::BTreeSet::new();
    let members: Vec<_> =
        keys.into_iter().filter(|key| seen.insert(key.clone())).take(CONTEXT_MEMBERS).map(|key| n.standing(community, Some(&key))).collect::<Result<_, _>>()?;
    let ledger: Vec<_> = n.ledger(community)?.into_iter().take(CONTEXT_LEDGER).collect();
    Ok(json!({
        "ok": true, "community": cv, "me": me, "posts": posts, "members": members, "ledger": ledger,
        "snapshotLimits": { "posts": CONTEXT_POSTS, "commentsPerPost": CONTEXT_COMMENTS, "members": CONTEXT_MEMBERS, "ledger": CONTEXT_LEDGER },
    }))
}

// ---------------------------------------------------------------------------- forge

fn do_forge(node: &mut Node, _f: &Flags, a: &[String], url: &str) -> ExitCode {
    if a.len() < 2 {
        return fail(format!("usage: forge <attack> <community-hex>   (attacks: {})", forge::catalogue().join(", ")));
    }
    let attack = a[0].clone();
    let community = match hoi_types::Id::from_hex(&a[1]) {
        Ok(c) => c,
        Err(_) => return fail("community must be a 64-char hex id"),
    };
    // The community's pow_bits, so forgeries clear the stamp and reach the stage they test.
    let bits = node.community(&a[1]).map(|c| c.limits.pow_bits).unwrap_or(8);
    let now = node.now();

    let forger = forge::Forger::new();
    let items = forge::forge(&attack, &forger, community, now, bits);

    // The local oracle: feed each forgery to our own node and record the pipeline's verdict.
    let mut report = Vec::new();
    let mut raw: Vec<Vec<u8>> = forger.credentials(now);
    for it in &items {
        if it.wire.is_empty() {
            return fail(it.note.clone());
        }
        let live = now.saturating_add(hoi_validate::CLOCK_SKEW) >= now; // forged time decides; see record
        let verdict = node.ingest(&it.wire, &Source::Peer { peer: "oracle".into(), live });
        let (accepted, detail) = match &verdict {
            Ok(_) => (true, "accepted (then held/contained — never forwarded if over standing)".to_string()),
            Err(e) => (false, format!("refused at: {}", e.code())),
        };
        let held = match it.expect {
            Expect::Refused => !accepted,
            Expect::Contained => true, // accepted-but-contained is fine; it must not spread
        };
        report.push(json!({
            "forgery": it.label,
            "expected": if it.expect == Expect::Refused { "refused" } else { "contained" },
            "local_verdict": detail,
            "held": held,
            "note": it.note,
        }));
        raw.push(it.wire.clone());
    }

    // Now inject the raw wire at the hub, as the attacker's tool would. The hub judges it too; the
    // referee confirms nothing leaked to honest members.
    let kept = block_on(wire::inject(node, url, raw));
    let all_held = report.iter().all(|r| r["held"].as_bool().unwrap_or(false));
    let res = json!({
        "ok": true,
        "attack": attack,
        "community": a[1],
        "injected_to_hub": kept.is_ok(),
        "hub_kept_link": kept.unwrap_or(false),
        "all_held_locally": all_held,
        "forgeries": report,
        "reading": "every forgery should be refused or contained; run the referee to confirm the honest network never shows it",
    });
    if all_held {
        out(res)
    } else {
        let _ = out(res);
        ExitCode::FAILURE
    }
}

// ---------------------------------------------------------------------------- referee

fn referee(rest: &[String]) -> ExitCode {
    let f = parse(rest);
    let mut node = match open(&f, "desktop") {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    if node.me().is_none() {
        // A referee needs an identity to sync; make a quiet one.
        let _ = node.create_identity(f.name.as_deref().unwrap_or("referee"));
        node.commit().ok();
    }
    let Some(url) = hub_url(&f, &node) else {
        return fail("no hub; pass --hub ws://… or run `agent … pair <link>` into the same --data first");
    };

    let once = |node: &mut Node| -> Result<Value, String> { block_on(wire::session(node, &url, |n| scoreboard(n, f.community.as_deref()))) };

    if !f.watch {
        return match once(&mut node) {
            Ok(v) => {
                if f.json {
                    out(v)
                } else {
                    print!("{}", render_scoreboard(&v));
                    ExitCode::SUCCESS
                }
            }
            Err(e) => fail(e),
        };
    }
    // Watch: re-sync and reprint every few seconds until interrupted.
    loop {
        match once(&mut node) {
            Ok(v) => {
                if f.json {
                    println!("{}", serde_json::to_string(&v).unwrap());
                } else {
                    print!("\x1b[2J\x1b[H{}", render_scoreboard(&v));
                }
            }
            Err(e) => eprintln!("referee: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

/// A health snapshot of every community this node is in (or just `only`).
fn scoreboard(n: &mut Node, only: Option<&str>) -> Value {
    let mut communities = Vec::new();
    for c in n.communities() {
        if !c.joined {
            continue;
        }
        if only.is_some_and(|o| c.id != o) {
            continue;
        }
        let cv = match n.community(&c.id) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let feed = n.feed(Some(&c.id), "fresh", None).unwrap_or_default();
        let napping = feed.iter().filter(|p| p.napping.is_some()).count();
        // Standing of everyone who has posted, so the board shows voice distribution.
        let mut people = std::collections::BTreeMap::new();
        for p in &feed {
            people.entry(p.author.key.clone()).or_insert(());
        }
        let mut rows = Vec::new();
        for key in people.keys() {
            if let Ok(s) = n.standing(&c.id, Some(key)) {
                rows.push(json!({
                    "key": s.person.short, "level": s.level, "voice": (s.voice * 1000.0).round() / 1000.0,
                    "blocked": s.blocked, "struck": s.struck,
                }));
            }
        }
        communities.push(json!({
            "id": c.id,
            "name": cv.name,
            "members": cv.established,
            "mods": cv.mods.len(),
            "banned": cv.banned.len(),
            "proposals": cv.proposals.len(),
            "posts_visible": feed.len() - napping,
            "posts_napping": napping,
            "people": rows,
        }));
    }
    json!({ "ok": true, "hub": n.net_status().label, "communities": communities })
}

fn render_scoreboard(v: &Value) -> String {
    let mut s = String::new();
    s.push_str(&format!("\n  hoi referee · {}\n", v["hub"].as_str().unwrap_or("")));
    s.push_str("  ──────────────────────────────────────────────────────────────\n");
    let empty = Vec::new();
    for c in v["communities"].as_array().unwrap_or(&empty) {
        s.push_str(&format!(
            "  {}  ({})\n    established {}  mods {}  banned {}  proposals pending {}\n    posts: {} showing, {} napping\n",
            c["name"].as_str().unwrap_or("?"),
            &c["id"].as_str().unwrap_or("")[..c["id"].as_str().map(|x| x.len().min(12)).unwrap_or(0)],
            c["members"],
            c["mods"],
            c["banned"],
            c["proposals"],
            c["posts_visible"],
            c["posts_napping"],
        ));
        for p in c["people"].as_array().unwrap_or(&empty) {
            let flags = if p["blocked"].as_bool() == Some(true) {
                " [blocked]"
            } else if p["struck"].as_bool() == Some(true) {
                " [struck]"
            } else {
                ""
            };
            s.push_str(&format!("      {:>14}  {:<12} voice {}{}\n", p["key"].as_str().unwrap_or(""), p["level"].as_str().unwrap_or(""), p["voice"], flags));
        }
    }
    s.push_str("  ──────────────────────────────────────────────────────────────\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoi_core::dto::{CommunityDraft, LadderPatch, LimitsPatch};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn participant(name: &str, clock: &Arc<AtomicU64>) -> Node {
        let clock = clock.clone();
        let mut node = Node::open(Box::new(hoi_store::Ephemeral), "desktop", WEB_BASE, Box::new(move || clock.load(Ordering::SeqCst))).unwrap();
        node.set_identity_bits(4);
        node.create_identity(name).unwrap();
        node
    }

    fn community() -> (Node, String, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(1_790_000_000));
        let mut owner = participant("Alex", &clock);
        let cv = owner
            .create_community(CommunityDraft {
                name: "Neighbours".into(),
                description: "A place to discuss the neighbourhood.".into(),
                color: "mint".into(),
                limits: Some(LimitsPatch {
                    pow_bits: Some(8),
                    ladder: Some(LadderPatch { identity_bits: Some(4), admit_vouches: Some(1), established_age: Some(0), ..Default::default() }),
                    ..Default::default()
                }),
                rules: Some("Be kind and explain your decisions.".into()),
                co_owners: None,
                threshold: None,
            })
            .unwrap();
        (owner, cv.id, clock)
    }

    fn share(from: &Node, to: &mut Node, community: &str) {
        to.import_bundle(&from.export_bundle(community).unwrap()).unwrap();
    }

    fn admitted_member(owner: &mut Node, community: &str, clock: &Arc<AtomicU64>) -> Node {
        let mut member = participant("Morgan", clock);
        member.join(&owner.invite(community).unwrap().uri).unwrap();
        share(owner, &mut member, community);
        // A post carries the new identity to the owner's ordinary import pipeline.
        let welcome = act(&mut member, "post", &[community.into(), "Introductions".into(), "I would like to help organise the next event.".into()]);
        assert_eq!(welcome["ok"], true, "{welcome}");
        share(&member, owner, community);
        owner.vouch(community, &member.me().unwrap().root, 1, None).unwrap();
        share(owner, &mut member, community);
        member
    }

    #[test]
    fn context_exposes_nested_conversation_and_real_votes() {
        let (mut owner, cid, clock) = community();
        let mut member = admitted_member(&mut owner, &cid, &clock);
        let post = act(&mut member, "post", &[cid.clone(), "Weekend plans".into(), "Should we organise a picnic?".into()]);
        let post_id = post["id"].as_str().unwrap().to_string();
        let comment = act(&mut member, "comment", &[post_id.clone(), "I can bring the food.".into()]);
        let comment_id = comment["id"].as_str().unwrap().to_string();
        share(&member, &mut owner, &cid);
        let reply = act(&mut owner, "comment", &[comment_id.clone(), "What time works for you?".into()]);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(act(&mut owner, "vote", &[post_id.clone(), "1".into()])["ok"], true);
        assert_eq!(act(&mut owner, "vote", &[comment_id.clone(), "-1".into()])["ok"], true);

        let context = act(&mut owner, "context", std::slice::from_ref(&cid));
        assert_eq!(context["ok"], true, "{context}");
        assert_eq!(context["community"]["role"], "owner");
        assert_eq!(context["me"]["person"]["key"], owner.standing(&cid, None).unwrap().person.key);
        let discussion = context["posts"].as_array().unwrap().iter().find(|p| p["id"] == post_id).unwrap();
        assert_eq!(discussion["text"], "Should we organise a picnic?");
        assert_eq!(discussion["score"], 1);
        assert_eq!(discussion["myVote"], 1);
        let comments = discussion["comments"].as_array().unwrap();
        assert!(comments.iter().any(|c| c["id"] == comment_id && c["myVote"] == -1 && c["score"] == -1));
        assert!(comments.iter().any(|c| c["id"] == reply["id"] && c["parent"] == comment_id && c["authorName"] == "Alex"));
        assert!(context["members"].as_array().unwrap().iter().any(|m| m["person"]["key"] == member.standing(&cid, None).unwrap().person.key));
        let thread = act(&mut owner, "thread", &[post_id]);
        assert_eq!(thread["post"]["comments"][0]["children"][0]["id"], reply["id"]);
    }

    #[test]
    fn rules_proposals_keep_owner_permissions_and_member_objections() {
        let (mut owner, cid, clock) = community();
        let mut member = admitted_member(&mut owner, &cid, &clock);
        let rules = "Community decisions require Morgan's approval.";
        let denied = act(&mut member, "propose-rules", &[cid.clone(), rules.into()]);
        assert_eq!(denied["ok"], false);
        assert!(denied["error"].as_str().unwrap().contains("owners"));

        let proposed = act(&mut owner, "propose-rules", &[cid.clone(), rules.into()]);
        assert_eq!(proposed["status"], "published", "{proposed}");
        let proposal_id = proposed["entry"]["id"].as_str().unwrap().to_string();
        share(&owner, &mut member, &cid);
        let veto = act(&mut member, "review", &[proposal_id.clone(), "veto".into(), "Decisions should remain shared.".into()]);
        assert_eq!(veto["ok"], true, "{veto}");
        share(&member, &mut owner, &cid);

        let context = act(&mut owner, "context", std::slice::from_ref(&cid));
        assert_ne!(context["community"]["rules"]["text"], rules);
        let proposal = context["community"]["proposals"].as_array().unwrap().iter().find(|p| p["id"] == proposal_id).unwrap();
        assert_eq!(proposal["objecting"][0]["reason"], "Decisions should remain shared.");
        let ledger = act(&mut owner, "ledger", &[cid]);
        assert!(ledger["entries"].as_array().unwrap().iter().any(|e| e["id"] == proposal_id));
    }

    #[test]
    fn context_bounds_history_and_preserves_recent_nested_replies() {
        let (mut owner, cid, clock) = community();
        let mut last_post = String::new();
        for i in 0..CONTEXT_POSTS + 1 {
            clock.fetch_add(1, Ordering::SeqCst);
            let post = act(&mut owner, "post", &[cid.clone(), format!("Discussion {i}"), "A useful neighbourhood update.".into()]);
            assert_eq!(post["ok"], true, "{post}");
            last_post = post["id"].as_str().unwrap().to_string();
        }
        let mut parent = last_post.clone();
        for i in 0..CONTEXT_COMMENTS + 1 {
            clock.fetch_add(1, Ordering::SeqCst);
            // The last two replies are nested; the recent one must survive truncation.
            let target = if i >= CONTEXT_COMMENTS - 1 { parent.clone() } else { last_post.clone() };
            let reply = act(&mut owner, "comment", &[target, format!("Follow-up question {i}.")]);
            assert_eq!(reply["ok"], true, "{reply}");
            parent = reply["id"].as_str().unwrap().to_string();
        }
        let context = act(&mut owner, "context", &[cid]);
        assert_eq!(context["posts"].as_array().unwrap().len(), CONTEXT_POSTS);
        let post = &context["posts"][0];
        assert_eq!(post["id"], last_post);
        assert_eq!(post["comments"].as_array().unwrap().len(), CONTEXT_COMMENTS);
        assert_eq!(post["omittedComments"], 1);
        assert!(post["comments"].as_array().unwrap().iter().any(|c| c["id"] == parent));
        assert_eq!(clipped("é🙂hello", 2), "é🙂…");
    }
}
