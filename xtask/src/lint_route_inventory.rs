//! Route inventory lint: verify every route in the axum Router is
//! documented in the route inventory.
//!
//! Parses all `.route()` calls in the axum Router definition and
//! compares against the `ROUTE_INVENTORY` in `src/http/mod.rs`.
//! Ensures every route is documented.

use std::path::Path;

use anyhow::{bail, Result};

/// Routes that are intentionally not in the inventory (e.g. fallbacks,
/// internal-only routes).
const INVENTORY_SKIP: &[&str] = &[
    "/openapi.json", // Served by the openapi module, not a business route
];

pub fn run(repo_root: &Path) -> Result<()> {
    let router_path = repo_root.join("crates/minos-backend/src/http/mod.rs");
    let v1_mod_path = repo_root.join("crates/minos-backend/src/http/v1/mod.rs");

    if !router_path.exists() {
        bail!("route-inventory: {} not found", router_path.display());
    }

    let router_src = std::fs::read_to_string(&router_path).unwrap_or_else(|_| String::new());
    let v1_src = std::fs::read_to_string(&v1_mod_path).unwrap_or_else(|_| String::new());

    // Extract .route("...") calls from the router definition.
    let mut registered_routes: Vec<String> = Vec::new();
    for line in router_src.lines().chain(v1_src.lines()) {
        let trimmed = line.trim();
        if trimmed.starts_with(".route(") || trimmed.starts_with(".route (") {
            if let Some(path) = extract_route_path(trimmed) {
                registered_routes.push(path);
            }
        }
    }

    // Extract routes from the ROUTE_INVENTORY array.
    let inventory_routes = extract_inventory_routes(&router_src);

    // Check that every registered route is in the inventory.
    let mut missing: Vec<String> = Vec::new();
    for route in &registered_routes {
        if INVENTORY_SKIP.iter().any(|s| route == s) {
            continue;
        }
        if !inventory_routes.iter().any(|inv| inv == route) {
            missing.push(route.clone());
        }
    }

    if missing.is_empty() {
        eprintln!(
            "route-inventory: all {} registered routes are in the inventory",
            registered_routes.len()
        );
        Ok(())
    } else {
        for m in &missing {
            eprintln!("route-inventory: missing from ROUTE_INVENTORY: {m}");
        }
        bail!(
            "route-inventory: {} registered route(s) not in ROUTE_INVENTORY",
            missing.len()
        );
    }
}

/// Extract the path string from a `.route("/path", ...)` call.
fn extract_route_path(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Extract path strings from the `ROUTE_INVENTORY` const definition.
fn extract_inventory_routes(src: &str) -> Vec<String> {
    let mut routes = Vec::new();
    let mut in_inventory = false;

    for line in src.lines() {
        let trimmed = line.trim();

        if trimmed.contains("ROUTE_INVENTORY") && trimmed.contains('&') {
            in_inventory = true;
            continue;
        }

        if in_inventory {
            if trimmed.starts_with("];") {
                break;
            }
            // Look for RouteContract::new("METHOD", "/path", ...)
            if let Some(method_start) = trimmed.find("RouteContract::new(") {
                let after = &trimmed[method_start + "RouteContract::new(".len()..];
                // Skip the method string, find the path string.
                let parts: Vec<&str> = after.splitn(4, '"').collect();
                if parts.len() >= 4 {
                    routes.push(parts[3].to_string());
                }
            }
        }
    }

    routes
}
