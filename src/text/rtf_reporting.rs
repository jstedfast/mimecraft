// Adapted from MimeKit's Text/RtfCappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

pub(super) enum RtfReport {
    Issue,
    Limit,
    Suppress,
}

#[derive(Debug)]
pub(super) struct RtfReportingLimits<const N: usize> {
    limit: usize,
    pub(super) counts: [usize; N],
    pub(super) reported_limit: bool,
}

impl<const N: usize> RtfReportingLimits<N> {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            counts: [0; N],
            reported_limit: false,
        }
    }

    // Call only when a callback is present: discarded issues do not spend quotas.
    pub(super) fn check(&mut self, index: usize) -> RtfReport {
        // A total budget would let a flood of cheap violations hide other
        // kinds that follow, so each defect has its own independent counter.
        if self.limit != 0
            && let Some(count) = self.counts.get_mut(index)
        {
            if *count == self.limit {
                // The report becomes incomplete at the first suppressed issue,
                // not merely on reaching the limit. Emit its location only once:
                // repeating markers for later kinds would reintroduce flooding.
                if !self.reported_limit {
                    self.reported_limit = true;
                    return RtfReport::Limit;
                }
                return RtfReport::Suppress;
            }
            // Only increment below the limit, even when the limit is usize::MAX.
            *count += 1;
        }
        // The marker is outside the dense defect range and has no quota.
        RtfReport::Issue
    }
}
