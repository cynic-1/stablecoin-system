use crate::types::*;

/// View change manager (Algorithm 3.7).
///
/// On timeout, nodes send NEW_VIEW messages. The new leader collects 2f+1
/// NEW_VIEW messages, selects the highest QC, and starts a new view.
pub struct ViewChangeManager {
    pub current_view: View,
    pub new_view_messages: Vec<NewViewMessage>,
    pub quorum_threshold: usize,
}

impl ViewChangeManager {
    pub fn new(current_view: View, quorum_threshold: usize) -> Self {
        ViewChangeManager {
            current_view,
            new_view_messages: Vec::new(),
            quorum_threshold,
        }
    }

    /// Create a NEW_VIEW message for a view transition.
    pub fn create_new_view(
        sender: NodeId,
        new_view: View,
        high_qc: Option<MacroQC>,
    ) -> NewViewMessage {
        let sig = Signature::sign(sender, &new_view.to_le_bytes());
        NewViewMessage {
            new_view,
            sender,
            high_qc,
            signature: sig,
        }
    }

    /// Process a NEW_VIEW message. Returns the new view's highest QC
    /// when quorum is reached.
    pub fn process_new_view(
        &mut self,
        msg: NewViewMessage,
    ) -> Option<(View, Option<MacroQC>)> {
        if msg.new_view != self.current_view + 1 {
            return None;
        }

        // Check for duplicate sender.
        if self.new_view_messages.iter().any(|m| m.sender == msg.sender) {
            return None;
        }

        self.new_view_messages.push(msg);

        if self.new_view_messages.len() >= self.quorum_threshold {
            // Select the highest QC among all messages.
            let best_qc = self
                .new_view_messages
                .iter()
                .filter_map(|m| m.high_qc.as_ref())
                .max_by_key(|qc| qc.height)
                .cloned();

            let new_view = self.current_view + 1;
            return Some((new_view, best_qc));
        }

        None
    }

    /// Compute view timeout with exponential backoff.
    pub fn view_timeout(
        base_timeout_ms: u64,
        view: View,
        rho: f64,
        max_timeout_ms: u64,
    ) -> u64 {
        let timeout = (base_timeout_ms as f64 * rho.powi(view as i32)) as u64;
        timeout.min(max_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_change_quorum() {
        let mut manager = ViewChangeManager::new(0, 3); // quorum = 3

        let msg0 = ViewChangeManager::create_new_view(0, 1, None);
        assert!(manager.process_new_view(msg0).is_none());

        let msg1 = ViewChangeManager::create_new_view(1, 1, None);
        assert!(manager.process_new_view(msg1).is_none());

        let msg2 = ViewChangeManager::create_new_view(2, 1, None);
        let result = manager.process_new_view(msg2);
        assert!(result.is_some());
        let (new_view, _) = result.unwrap();
        assert_eq!(new_view, 1);
    }

    #[test]
    fn test_view_change_selects_highest_qc() {
        let mut manager = ViewChangeManager::new(0, 2);

        // Create fake QCs at different heights.
        let header_low = MacroHeader {
            view: 0, height: 1, leader: 0,
            parent_qc: None, slot_entries: vec![],
            digest: DigestBytes::hash(b"low"),
        };
        let qc_low = MacroQC {
            view: 0, height: 1, header_digest: header_low.digest,
            header: header_low, votes: vec![],
        };

        let header_high = MacroHeader {
            view: 0, height: 5, leader: 0,
            parent_qc: None, slot_entries: vec![],
            digest: DigestBytes::hash(b"high"),
        };
        let qc_high = MacroQC {
            view: 0, height: 5, header_digest: header_high.digest,
            header: header_high, votes: vec![],
        };

        let msg0 = ViewChangeManager::create_new_view(0, 1, Some(qc_low));
        manager.process_new_view(msg0);

        let msg1 = ViewChangeManager::create_new_view(1, 1, Some(qc_high.clone()));
        let result = manager.process_new_view(msg1);
        let (_, best_qc) = result.unwrap();
        assert_eq!(best_qc.unwrap().height, 5);
    }

    #[test]
    fn test_duplicate_sender_rejected() {
        let mut manager = ViewChangeManager::new(0, 3);

        let msg = ViewChangeManager::create_new_view(0, 1, None);
        manager.process_new_view(msg.clone());
        manager.process_new_view(msg); // duplicate
        assert_eq!(manager.new_view_messages.len(), 1);
    }

    #[test]
    fn test_view_timeout_exponential() {
        let t0 = ViewChangeManager::view_timeout(5000, 0, 1.5, 60000);
        let t1 = ViewChangeManager::view_timeout(5000, 1, 1.5, 60000);
        let t2 = ViewChangeManager::view_timeout(5000, 2, 1.5, 60000);

        assert_eq!(t0, 5000);
        assert_eq!(t1, 7500);
        assert!(t2 > t1);

        // Check max cap.
        let t_large = ViewChangeManager::view_timeout(5000, 100, 1.5, 60000);
        assert_eq!(t_large, 60000);
    }
}
