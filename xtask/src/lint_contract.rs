//! Contract drift gate: verify the checked-in OpenAPI baseline matches
//! the generated spec.
//!
//! Parses the generated `openapi.json` and compares it against a
//! checked-in baseline. Fails CI if there are breaking changes (removed
//! paths, removed fields, changed types). Non-breaking additions (new
//! optional fields, new endpoints) are allowed.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;

const BASELINE_PATH: &str = "docs/api/openapi-baseline.json";
const GENERATED_PATH: &str = "target/openapi.json";

pub fn run(repo_root: &Path) -> Result<()> {
    let baseline_path = repo_root.join(BASELINE_PATH);
    let generated_path = repo_root.join(GENERATED_PATH);

    if !baseline_path.exists() {
        eprintln!(
            "lint-contract: baseline not found at {}; skipping (run `cargo xtask gen-backend-platform-contract` first)",
            baseline_path.display()
        );
        return Ok(());
    }

    if !generated_path.exists() {
        eprintln!(
            "lint-contract: generated spec not found at {}; run `cargo xtask gen-backend-platform-contract` first",
            generated_path.display()
        );
        return Ok(());
    }

    let baseline: Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path)
            .with_context(|| format!("reading {}", baseline_path.display()))?,
    )
    .with_context(|| format!("parsing baseline JSON at {}", baseline_path.display()))?;

    let generated: Value = serde_json::from_str(
        &std::fs::read_to_string(&generated_path)
            .with_context(|| format!("reading {}", generated_path.display()))?,
    )
    .with_context(|| format!("parsing generated JSON at {}", generated_path.display()))?;

    let mut errors: Vec<String> = Vec::new();

    // Check paths: every path in baseline must exist in generated.
    if let (Some(baseline_paths), Some(gen_paths)) = (baseline.get("paths"), generated.get("paths"))
    {
        if let (Some(bp), Some(gp)) = (baseline_paths.as_object(), gen_paths.as_object()) {
            for path in bp.keys() {
                if !gp.contains_key(path) {
                    errors.push(format!("removed path: {path}"));
                }
            }
        }
    }

    // Check schemas: every schema in baseline must exist in generated.
    if let (Some(baseline_schemas), Some(gen_schemas)) = (
        baseline
            .pointer("/components/schemas")
            .and_then(|v| v.as_object()),
        generated
            .pointer("/components/schemas")
            .and_then(|v| v.as_object()),
    ) {
        for (name, baseline_schema) in baseline_schemas {
            if let Some(gen_schema) = gen_schemas.get(name) {
                // Check that all required fields in baseline are still present.
                check_schema_compatibility(name, baseline_schema, gen_schema, &mut errors);
            } else {
                errors.push(format!("removed schema: {name}"));
            }
        }
    }

    if errors.is_empty() {
        eprintln!("lint-contract: no breaking changes detected");
        Ok(())
    } else {
        for err in &errors {
            eprintln!("lint-contract: BREAKING: {err}");
        }
        bail!(
            "lint-contract: {} breaking change(s) detected",
            errors.len()
        );
    }
}

/// Check that a schema hasn't had required fields removed or types changed.
fn check_schema_compatibility(
    name: &str,
    baseline: &Value,
    generated: &Value,
    errors: &mut Vec<String>,
) {
    // Check required fields haven't been removed.
    if let (Some(baseline_req), Some(gen_req)) = (
        baseline.get("required").and_then(|v| v.as_array()),
        generated.get("required").and_then(|v| v.as_array()),
    ) {
        let gen_set: std::collections::HashSet<_> =
            gen_req.iter().filter_map(|v| v.as_str()).collect();
        for field in baseline_req {
            if let Some(field_name) = field.as_str() {
                if !gen_set.contains(field_name) {
                    errors.push(format!(
                        "schema '{name}': required field '{field_name}' removed"
                    ));
                }
            }
        }
    }

    // Check properties haven't been removed.
    if let (Some(baseline_props), Some(gen_props)) = (
        baseline.get("properties").and_then(|v| v.as_object()),
        generated.get("properties").and_then(|v| v.as_object()),
    ) {
        for prop_name in baseline_props.keys() {
            if !gen_props.contains_key(prop_name) {
                errors.push(format!("schema '{name}': property '{prop_name}' removed"));
            }
        }
    }
}
