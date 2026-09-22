use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NotificationIdentity {
    pub id: u32,
    generation: u64,
}

#[derive(Default)]
pub(super) struct NotificationRegistry {
    next_generation: u64,
    entries: HashMap<String, NotificationIdentity>,
}

impl NotificationRegistry {
    pub fn get(&self, key: &str) -> Option<NotificationIdentity> {
        self.entries.get(key).copied()
    }

    pub fn replace(&mut self, key: String, id: u32) -> NotificationIdentity {
        self.next_generation += 1;
        let identity = NotificationIdentity {
            id,
            generation: self.next_generation,
        };
        self.entries.insert(key, identity);
        identity
    }

    pub fn remove_if_current(&mut self, key: &str, identity: NotificationIdentity) {
        if self.get(key) == Some(identity) {
            self.entries.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_close_cannot_remove_replacement_with_reused_os_id() {
        let mut registry = NotificationRegistry::default();
        let old = registry.replace("request".into(), 7);
        let replacement = registry.replace("request".into(), 7);
        registry.remove_if_current("request", old);
        assert_eq!(registry.get("request"), Some(replacement));
        assert_ne!(
            registry.get("request"),
            Some(old),
            "stale callbacks must not activate replacement actions"
        );
    }

    #[test]
    fn completed_removal_does_not_erase_a_new_notification() {
        let mut registry = NotificationRegistry::default();
        let closing = registry.replace("request".into(), 7);
        let replacement = registry.replace("request".into(), 8);
        registry.remove_if_current("request", closing);
        assert_eq!(registry.get("request"), Some(replacement));
        registry.remove_if_current("request", replacement);
        assert_eq!(registry.get("request"), None);
    }
}
