/// Cumulative non-`final` tool-call budget.
pub struct ToolBudget {
    pub max: Option<u32>,
    pub used: u32,
    pub wake_sent: bool,
}

impl ToolBudget {
    pub fn new(max: Option<u32>) -> Self {
        Self {
            max,
            used: 0,
            wake_sent: false,
        }
    }

    pub fn allows(&self, name: &str) -> bool {
        if name == "final" {
            return true;
        }
        match self.max {
            None => true,
            Some(max) => self.used < max,
        }
    }

    pub fn record(&mut self, name: &str) {
        if name != "final" {
            self.used += 1;
        }
    }

    pub fn exhausted(&self) -> bool {
        matches!(self.max, Some(max) if self.used >= max)
    }

    pub fn final_only(&self) -> bool {
        self.exhausted()
    }

    pub fn wake_once(&mut self) -> bool {
        if self.exhausted() && !self.wake_sent {
            self.wake_sent = true;
            true
        } else {
            false
        }
    }
}

impl Default for ToolBudget {
    fn default() -> Self {
        Self::new(None)
    }
}
