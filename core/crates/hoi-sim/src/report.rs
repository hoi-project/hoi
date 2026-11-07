//! The scorecard a battle prints: one line per protocol invariant, each either held or broke,
//! with the measured numbers behind it, and an overall verdict. Serializable so a referee or a
//! script can read it as JSON.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// What the protocol is supposed to guarantee.
    pub invariant: String,
    /// Did it hold?
    pub held: bool,
    /// The measured evidence, in plain words.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Scorecard {
    pub scenario: String,
    pub honest: usize,
    pub attackers: usize,
    pub days: u64,
    pub checks: Vec<Check>,
    /// Free-form counters for the curious (forged injected, spam posted, …).
    pub tallies: Vec<(String, i64)>,
}

impl Scorecard {
    pub fn new(scenario: &str, honest: usize, attackers: usize, days: u64) -> Scorecard {
        Scorecard { scenario: scenario.into(), honest, attackers, days, checks: Vec::new(), tallies: Vec::new() }
    }

    pub fn check(&mut self, invariant: &str, held: bool, detail: impl Into<String>) {
        self.checks.push(Check { invariant: invariant.into(), held, detail: detail.into() });
    }

    pub fn tally(&mut self, name: &str, n: i64) {
        self.tallies.push((name.into(), n));
    }

    pub fn held(&self) -> bool {
        self.checks.iter().all(|c| c.held)
    }

    /// A human-readable scoreboard.
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "\n  hoi protocol siege · scenario: {} · {} honest, {} adversarial · {} days simulated\n",
            self.scenario, self.honest, self.attackers, self.days
        ));
        s.push_str("  ──────────────────────────────────────────────────────────────────────────\n");
        for c in &self.checks {
            let mark = if c.held { "✓ held " } else { "✗ BROKE" };
            s.push_str(&format!("  {mark}  {}\n            {}\n", c.invariant, c.detail));
        }
        if !self.tallies.is_empty() {
            s.push_str("  ──────────────────────────────────────────────────────────────────────────\n");
            let line: Vec<String> = self.tallies.iter().map(|(k, v)| format!("{k}={v}")).collect();
            s.push_str(&format!("  counters: {}\n", line.join("  ")));
        }
        s.push_str("  ──────────────────────────────────────────────────────────────────────────\n");
        let n = self.checks.len();
        let held = self.checks.iter().filter(|c| c.held).count();
        if self.held() {
            s.push_str(&format!("  VERDICT: the protocol held. {held}/{n} invariants stood against the attack.\n"));
        } else {
            s.push_str(&format!("  VERDICT: the protocol BROKE. {held}/{n} invariants held; see ✗ above.\n"));
        }
        s
    }
}
