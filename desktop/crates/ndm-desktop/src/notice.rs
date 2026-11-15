use std::time::{Duration, Instant};

const LIFETIME: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct Notice {
    current: Option<(String, Instant)>,
}

impl Notice {
    pub fn show(&mut self, message: String, now: Instant) {
        self.current = Some((message, now + LIFETIME));
    }

    pub fn message(&self) -> Option<&str> {
        self.current.as_ref().map(|(message, _)| message.as_str())
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    pub fn expire(&mut self, now: Instant) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|(_, deadline)| now >= *deadline)
        {
            self.clear();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_notice_disappears_without_further_user_actions() {
        let now = Instant::now();
        let mut notice = Notice::default();
        notice.show("Deleted 1 download".into(), now);
        assert!(!notice.expire(now + Duration::from_secs(4)));
        assert_eq!(notice.message(), Some("Deleted 1 download"));
        assert!(notice.expire(now + Duration::from_secs(5)));
        assert_eq!(notice.message(), None);
    }

    #[test]
    fn old_timeout_does_not_clear_a_replacement_notice() {
        let now = Instant::now();
        let mut notice = Notice::default();
        notice.show("Deleted 1 download".into(), now);
        notice.show("Deleted 1 download".into(), now + Duration::from_secs(3));
        assert!(!notice.expire(now + Duration::from_secs(5)));
        assert_eq!(notice.message(), Some("Deleted 1 download"));
        assert!(notice.expire(now + Duration::from_secs(8)));
        notice.show("Another notice".into(), now + Duration::from_secs(9));
        notice.clear();
        assert!(!notice.expire(now + Duration::from_secs(20)));
        assert_eq!(notice.message(), None);
    }
}
