use serde_json::{Map, Value};

use super::super::languages::Languages;
use super::super::manifest::Runtime;
use super::PackageError;
use super::errors::service_error;
use crate::l10n::text;

pub(super) fn services(
    value: Option<Value>,
    runtime: Runtime,
    langs: &Languages,
) -> Result<Map<String, Value>, PackageError> {
    match (runtime, value) {
        (Runtime::Compose, Some(Value::Object(services))) if !services.is_empty() => Ok(services),
        (Runtime::Compose, _) => Err(service_error(
            langs,
            "*",
            text(langs, "plugin-install-pkg-needs-service", &[]),
        )),
        (_, None) => Ok(Map::new()),
        (_, Some(_)) => Err(service_error(
            langs,
            "*",
            text(
                langs,
                "plugin-install-runtime-services",
                &[("runtime", runtime.as_str().into())],
            ),
        )),
    }
}
