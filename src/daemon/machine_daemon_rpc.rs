use crate::daemon::CorrelationId;
use crate::daemon::DaemonLogWireEvent;
use crate::daemon::LogStreamRequest;
use crate::daemon::MachineError;
use crate::daemon::PingResponse;
use crate::daemon::QueryResponse;
use crate::daemon::StatusRequest;
use crate::daemon::StatusResponse;
use crate::daemon::SyncRequest;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;

#[vox::service]
pub trait MachineDaemonRpc {
    async fn ping(&self, logs: vox::Tx<DaemonLogWireEvent>) -> Result<PingResponse, MachineError>;

    async fn shutdown(&self, logs: vox::Tx<DaemonLogWireEvent>) -> Result<(), MachineError>;

    async fn query(
        &self,
        request: QueryPlan,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<QueryResponse, MachineError>;

    async fn query_stream(
        &self,
        request: QueryPlan,
        rows: vox::Tx<QueryResultRow>,
        logs: vox::Tx<DaemonLogWireEvent>,
        cancel: vox::Rx<u8>,
    ) -> Result<CorrelationId, MachineError>;

    async fn sync(
        &self,
        request: SyncRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<(), MachineError>;

    async fn status(
        &self,
        request: StatusRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<StatusResponse, MachineError>;

    async fn stream_logs(
        &self,
        request: LogStreamRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
        cancel: vox::Rx<u8>,
    ) -> Result<(), MachineError>;
}
