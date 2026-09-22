//! Content-free progress of a user-confirmed, frozen group of stored SMS fragments.
use dji4g_domain::{SmsDeleteItemResult, SmsFragmentKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmsDeleteItemSnapshot {
    pub fragment: SmsFragmentKey,
    pub result: SmsDeleteItemResult,
    pub code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmsDeleteSnapshot {
    pub request_id: u64,
    pub total: usize,
    pub finished: bool,
    pub items: Vec<SmsDeleteItemSnapshot>,
}

impl SmsDeleteSnapshot {
    pub(crate) fn new(request_id: u64, fragments: Vec<SmsFragmentKey>) -> Self {
        Self {
            request_id,
            total: fragments.len(),
            finished: false,
            items: fragments
                .into_iter()
                .map(|fragment| SmsDeleteItemSnapshot {
                    fragment,
                    result: SmsDeleteItemResult::NotAttempted,
                    code: None,
                })
                .collect(),
        }
    }
}
