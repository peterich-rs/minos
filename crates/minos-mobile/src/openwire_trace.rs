use std::net::SocketAddr;
use std::sync::Arc;

use http::{Method, Request, Response};
use openwire::{
    CallContext, EventListener, EventListenerFactory, LogLevel, LoggerInterceptor, RequestBody,
    ResponseBody, WireError,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenwireTraceFactory {
    component: &'static str,
}

impl OpenwireTraceFactory {
    pub(crate) const fn new(component: &'static str) -> Self {
        Self { component }
    }
}

pub(crate) fn logger_interceptor(component: &'static str) -> LoggerInterceptor {
    LoggerInterceptor::with_logger(LogLevel::Body, move |message: &str| {
        tracing::info!(
            target: "minos_mobile::network",
            component,
            message = %message,
            "openwire http"
        );
    })
}

impl EventListenerFactory for OpenwireTraceFactory {
    fn create(&self, request: &Request<RequestBody>) -> Arc<dyn EventListener> {
        Arc::new(OpenwireTraceListener {
            component: self.component,
            method: request.method().clone(),
            uri: request.uri().to_string(),
        })
    }
}

#[derive(Debug)]
struct OpenwireTraceListener {
    component: &'static str,
    method: Method,
    uri: String,
}

impl EventListener for OpenwireTraceListener {
    fn call_start(&self, ctx: &CallContext, _request: &Request<RequestBody>) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            "openwire call start"
        );
    }

    fn call_end(&self, ctx: &CallContext) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            "openwire call complete"
        );
    }

    fn call_failed(&self, ctx: &CallContext, error: &WireError) {
        tracing::warn!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            response_status = ?error.response_status().map(|status| status.as_u16()),
            error = %error,
            "openwire call failed"
        );
    }

    fn dns_start(&self, ctx: &CallContext, host: &str, port: u16) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            host,
            port,
            "openwire dns start"
        );
    }

    fn dns_end(&self, ctx: &CallContext, host: &str, addrs: &[SocketAddr]) {
        let resolved = addrs
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            host,
            resolved = %resolved,
            "openwire dns end"
        );
    }

    fn dns_failed(&self, ctx: &CallContext, host: &str, error: &WireError) {
        tracing::warn!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            host,
            error = %error,
            "openwire dns failed"
        );
    }

    fn route_plan(&self, ctx: &CallContext, route_count: usize, fast_fallback_enabled: bool) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            route_count,
            fast_fallback_enabled,
            "openwire route plan"
        );
    }

    fn connect_start(&self, ctx: &CallContext, addr: SocketAddr) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            addr = %addr,
            "openwire connect start"
        );
    }

    fn connect_failed(&self, ctx: &CallContext, addr: SocketAddr, error: &WireError) {
        tracing::warn!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            addr = %addr,
            error = %error,
            "openwire connect failed"
        );
    }

    fn response_headers_end(&self, ctx: &CallContext, response: &Response<ResponseBody>) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            status = %response.status(),
            "openwire response headers"
        );
    }
}
