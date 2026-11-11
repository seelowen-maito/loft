//! Port ranges: the fixed block of local ports each agent serves its dev servers on.
//!
//! Slot `n` is ports `FIRST + n * SIZE` to `FIRST + n * SIZE + SIZE - 1`. An agent keeps its slot across runs and
//! restarts, so links the user saved keep working; deleting the agent frees it for the next one. Slots run from
//! well above the ports dev tools default to (3000, 5173, 8000) up to below macOS's ephemeral ports, and a slot
//! holding one of Loft's own ports is never handed out.

use crate::state::AppState;
use anyhow::{anyhow, Result};
use std::ops::RangeInclusive;

/// Ports per agent: a web app, its API and a few spares.
pub const SIZE: u16 = 10;
const FIRST: u16 = 20000;
/// macOS hands out ports from 49152 up to outgoing connections.
const END: u16 = 49152;
/// The variable holding the whole range (`20000-20009`); `PORT` holds its first port.
pub const RANGE_VAR: &str = "LOFT_PORT_RANGE";

fn ports(slot: i64) -> RangeInclusive<u16> {
    let first = FIRST + slot as u16 * SIZE;
    first..=first + SIZE - 1
}

/// The ports this Loft serves on: its API and its web app (Vite's, while developing Loft).
fn loft_ports(state: &AppState) -> Vec<u16> {
    let mut out = vec![state.cfg.port, state.cfg.port.wrapping_add(1), 7420, 7421];
    if let Some(p) = std::env::var("LOFT_WEB_PORT").ok().and_then(|p| p.parse().ok()) {
        out.push(p);
    }
    out
}

/// The agent's port range, given it the first time it needs one.
pub fn range(state: &AppState, agent_id: &str) -> Result<RangeInclusive<u16>> {
    let loft = loft_ports(state);
    let slots = ((END - FIRST) / SIZE) as i64;
    let usable = |slot: i64| slot < slots && !loft.iter().any(|p| ports(slot).contains(p));
    let slot = state.db.port_slot(agent_id, usable, slots)?.ok_or_else(|| anyhow!("every port range is taken"))?;
    Ok(ports(slot))
}

/// `20000-20009`, as `LOFT_PORT_RANGE` and the prompt give it.
pub fn describe(r: &RangeInclusive<u16>) -> String {
    format!("{}-{}", r.start(), r.end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn each_agent_keeps_its_own_range_and_a_deleted_agent_s_goes_to_the_next() {
        let st = crate::tasks::tests::state();
        let a = range(&st, "a").unwrap();
        let b = range(&st, "b").unwrap();
        assert_eq!(a, 20000..=20009);
        assert_eq!(b, 20010..=20019);
        // The same on every call, and after a restart (a new connection to the same db).
        assert_eq!(range(&st, "a").unwrap(), a);
        let reopened = crate::db::Db::open(&st.cfg.storage_dir.parent().unwrap().join("loft.db")).unwrap();
        assert_eq!(reopened.port_slot("b", |_| true, 10).unwrap(), Some(1));

        st.db.delete_agent("a").unwrap();
        assert_eq!(range(&st, "c").unwrap(), a, "a deleted agent's range is free again");
        assert_eq!(range(&st, "b").unwrap(), b);
    }

    #[test]
    fn a_range_holding_one_of_loft_s_ports_is_never_handed_out() {
        let mut st = crate::tasks::tests::state();
        let a = range(&st, "a").unwrap();
        // Loft now serves inside a's range (and its web app one port up, in the next): both move on.
        st.cfg = Arc::new(crate::state::Config { port: 20009, storage_dir: st.cfg.storage_dir.clone(), xcrun: st.cfg.xcrun.clone(), xcode: st.cfg.xcode.clone() });
        let moved = range(&st, "a").unwrap();
        assert!(!moved.contains(&20009) && !moved.contains(&20010), "{moved:?}");
        assert_ne!(moved, a);
        let b = range(&st, "b").unwrap();
        for r in [&moved, &b] {
            assert!(!loft_ports(&st).iter().any(|p| r.contains(p)), "{r:?}");
        }
        assert!(b.end() < moved.start() || moved.end() < b.start(), "{moved:?} and {b:?} overlap");
    }
}
