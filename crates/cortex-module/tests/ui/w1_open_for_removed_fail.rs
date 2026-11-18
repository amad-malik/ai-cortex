// Wave 1 ports2 (W1-33): `open_for` is no longer a method of `HandoffsApi`.
use async_trait::async_trait;
use cortex_db::ModuleTx;
use cortex_module::{HandoffView, HandoffsApi, PortError};
use cortex_types::WorkerId;

struct Old;

#[async_trait]
impl HandoffsApi for Old {
    async fn claimed_by(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(vec![])
    }
    async fn pending_for(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(vec![])
    }
    // The old mixed, capped list: no longer a method of the trait.
    async fn open_for(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(vec![])
    }
}

fn main() {}
