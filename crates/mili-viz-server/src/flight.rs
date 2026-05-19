//! Phase 4 M6 — the Arrow Flight bulk-geometry transport.
//!
//! A real `arrow.flight.protocol.FlightService` co-served next to
//! `MiliViz` on the same TCP port (phase-4-m6.md Decisions 26 & 27).
//! It is a thin adapter over the **same** in-process geometry store
//! the in-process `VizService::fetch_geometry` reads: `DoGet` resolves
//! the frozen `GeometryRef.flight_ticket` (the `geom:{seq}` bytes
//! assigned in phase-4-m2.md Decision 10) and streams the
//! **byte-identical** M2/M3 `MVG1`/`MVG2` blob as `FlightData`.
//!
//! Only `DoGet` is implemented; every other Flight RPC returns
//! `UNIMPLEMENTED` — the same frozen-stub discipline phase-4-m1.md
//! Decision 7 applies to the unused agent RPCs. The blob is an opaque
//! self-describing buffer (phase-4-m2.md Decision 11), never an Arrow
//! `RecordBatch`, so it rides verbatim in `FlightData.data_body`.

use std::pin::Pin;
use std::sync::Arc;

use mili_viz_proto::flight as fpb;
use tokio_stream::Stream;
use tonic::{Request, Response, Status, Streaming};

use crate::Inner;

type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

const FLIGHT_UNIMPLEMENTED: &str =
    "mili-viz Flight serves only DoGet (bulk geometry by the frozen \
     GeometryRef.flight_ticket); other Flight RPCs are unimplemented \
     (phase-4-m6.md Decision 26)";

/// Adapter implementing the canonical `FlightService` over the shared
/// session's in-process geometry store. Construct via
/// [`crate::VizService::flight_service`].
#[derive(Clone)]
pub struct FlightGeometryService {
    inner: Arc<Inner>,
}

impl FlightGeometryService {
    pub(crate) fn new(inner: Arc<Inner>) -> Self {
        Self { inner }
    }
}

#[tonic::async_trait]
impl fpb::flight_service_server::FlightService for FlightGeometryService {
    type DoGetStream = BoxStream<fpb::FlightData>;

    /// Resolve the frozen geometry ticket to its byte-stable
    /// `MVG1`/`MVG2` blob and stream it verbatim. The blob is the
    /// *exact* bytes `VizService::fetch_geometry` returns for the same
    /// ticket — M6 is a transport swap, not a format change
    /// (phase-4-m2.md Decision 10 / phase-4-m6.md Decision 26). The
    /// client concatenates `data_body` across the stream, so
    /// single-message vs. chunked framing is a transparent detail;
    /// at current corpus sizes a single message is sufficient.
    async fn do_get(
        &self,
        request: Request<fpb::Ticket>,
    ) -> Result<Response<Self::DoGetStream>, Status> {
        let ticket = request.into_inner().ticket;
        // The conventional result-catalog ticket (`phase-5-m3.md`
        // Decision 67) is served from the same session lock as
        // geometry — no `.proto` change, the existing `DoGet` bulk
        // boundary carries it. Geometry tickets are `geom:{seq}`.
        let blob = {
            let session = self.inner.session.lock().unwrap();
            if ticket == crate::CATALOG_TICKET {
                session.catalog_blob()
            } else {
                session.geom.get(&ticket).cloned()
            }
        };
        match blob {
            Some(blob) => {
                let data = fpb::FlightData {
                    flight_descriptor: None,
                    data_header: Vec::new(),
                    app_metadata: Vec::new(),
                    data_body: blob,
                };
                Ok(Response::new(Box::pin(tokio_stream::once(Ok(data)))))
            }
            None => Err(Status::not_found(format!(
                "unknown flight ticket ({} bytes); geometry tickets are \
                 server-assigned and resolve only on the issuing session",
                ticket.len()
            ))),
        }
    }

    type HandshakeStream = BoxStream<fpb::HandshakeResponse>;
    async fn handshake(
        &self,
        _request: Request<Streaming<fpb::HandshakeRequest>>,
    ) -> Result<Response<Self::HandshakeStream>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    type ListFlightsStream = BoxStream<fpb::FlightInfo>;
    async fn list_flights(
        &self,
        _request: Request<fpb::Criteria>,
    ) -> Result<Response<Self::ListFlightsStream>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    async fn get_flight_info(
        &self,
        _request: Request<fpb::FlightDescriptor>,
    ) -> Result<Response<fpb::FlightInfo>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    async fn poll_flight_info(
        &self,
        _request: Request<fpb::FlightDescriptor>,
    ) -> Result<Response<fpb::PollInfo>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    async fn get_schema(
        &self,
        _request: Request<fpb::FlightDescriptor>,
    ) -> Result<Response<fpb::SchemaResult>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    type DoPutStream = BoxStream<fpb::PutResult>;
    async fn do_put(
        &self,
        _request: Request<Streaming<fpb::FlightData>>,
    ) -> Result<Response<Self::DoPutStream>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    type DoExchangeStream = BoxStream<fpb::FlightData>;
    async fn do_exchange(
        &self,
        _request: Request<Streaming<fpb::FlightData>>,
    ) -> Result<Response<Self::DoExchangeStream>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    type DoActionStream = BoxStream<fpb::Result>;
    async fn do_action(
        &self,
        _request: Request<fpb::Action>,
    ) -> Result<Response<Self::DoActionStream>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }

    type ListActionsStream = BoxStream<fpb::ActionType>;
    async fn list_actions(
        &self,
        _request: Request<fpb::Empty>,
    ) -> Result<Response<Self::ListActionsStream>, Status> {
        Err(Status::unimplemented(FLIGHT_UNIMPLEMENTED))
    }
}
