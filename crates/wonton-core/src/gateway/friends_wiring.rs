use std::sync::Arc;

use super::Gateway;
use crate::friends::Friends;
use crate::matrix::FederationBlocks;

impl Gateway {
    #[must_use]
    pub fn with_people_limiter(mut self, limits: Arc<super::people_limits::PeopleLimiter>) -> Self {
        self.people_limits = limits;
        self
    }

    #[must_use]
    pub fn with_friends(mut self, friends: Option<Arc<Friends>>) -> Self {
        self.friends = friends;
        self
    }

    #[must_use]
    pub fn with_federation_blocks(mut self, blocks: Option<Arc<FederationBlocks>>) -> Self {
        self.federation_blocks = blocks;
        self
    }

    pub(crate) fn friends(&self) -> Option<Arc<Friends>> {
        self.friends.clone()
    }
}
