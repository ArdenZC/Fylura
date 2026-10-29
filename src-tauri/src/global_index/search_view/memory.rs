use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub(super) const DEFAULT_TOTAL_BUDGET_BYTES: usize = 256 * 1024 * 1024;
pub(super) const MAX_ACTIVE_VIEW_BYTES: usize = 160 * 1024 * 1024;

pub(super) struct MemoryBudget {
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
}

impl MemoryBudget {
    pub(super) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        })
    }

    #[cfg(test)]
    pub(super) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
}

pub(super) struct BudgetCharge {
    budget: Arc<MemoryBudget>,
    bytes: AtomicUsize,
}

impl BudgetCharge {
    pub(super) fn empty(budget: Arc<MemoryBudget>) -> Arc<Self> {
        Arc::new(Self {
            budget,
            bytes: AtomicUsize::new(0),
        })
    }

    pub(super) fn reserved(budget: Arc<MemoryBudget>, bytes: usize) -> Option<Arc<Self>> {
        let charge = Self::empty(budget);
        if charge.grow(bytes) {
            Some(charge)
        } else {
            None
        }
    }

    /// Reserve conservative accounted memory before an owned allocation.
    pub(super) fn grow(&self, bytes: usize) -> bool {
        if bytes == 0 {
            return true;
        }
        let mut used = self.budget.used.load(Ordering::Acquire);
        loop {
            let Some(next) = used.checked_add(bytes) else {
                return false;
            };
            if next > self.budget.limit {
                return false;
            }
            match self.budget.used.compare_exchange_weak(
                used,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let mut peak = self.budget.peak.load(Ordering::Acquire);
                    while next > peak {
                        match self.budget.peak.compare_exchange_weak(
                            peak,
                            next,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => break,
                            Err(observed) => peak = observed,
                        }
                    }
                    self.bytes.fetch_add(bytes, Ordering::AcqRel);
                    return true;
                }
                Err(observed) => used = observed,
            }
        }
    }

    pub(super) fn shrink(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let released = self
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_sub(bytes))
            });
        if let Ok(previous) = released {
            self.budget
                .used
                .fetch_sub(previous.min(bytes), Ordering::AcqRel);
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }
}

impl Drop for BudgetCharge {
    fn drop(&mut self) {
        self.budget
            .used
            .fetch_sub(self.bytes.load(Ordering::Acquire), Ordering::AcqRel);
    }
}
