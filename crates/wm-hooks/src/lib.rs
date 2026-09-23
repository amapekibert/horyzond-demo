//! Hook dispatch policy independent of scripting and compositor mutation.

/// Distinguishes lifecycle hooks from committed runtime events.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookEvent {
    Startup,
    Reload,
    Committed,
}
/// A quota-enforced dispatcher that refuses recursive evaluation.
#[derive(Debug)]
pub struct HookDispatcher {
    limit: usize,
    dispatched: usize,
    active: bool,
}
impl HookDispatcher {
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            dispatched: 0,
            active: false,
        }
    }
    /// Begins one committed-event dispatch.
    pub fn begin(&mut self, event: HookEvent) -> bool {
        if self.active || self.dispatched >= self.limit {
            return false;
        }
        self.active = true;
        if matches!(
            event,
            HookEvent::Startup | HookEvent::Reload | HookEvent::Committed
        ) {
            self.dispatched += 1;
        }
        true
    }
    /// Finishes the current dispatch before later committed events are considered.
    pub fn finish(&mut self) {
        self.active = false;
    }
    #[must_use]
    pub const fn dispatched(&self) -> usize {
        self.dispatched
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotas_and_recursion_are_bounded() {
        let mut hooks = HookDispatcher::new(1);
        assert!(hooks.begin(HookEvent::Startup));
        assert!(!hooks.begin(HookEvent::Committed));
        hooks.finish();
        assert!(!hooks.begin(HookEvent::Reload));
        assert_eq!(hooks.dispatched(), 1);
    }
}
