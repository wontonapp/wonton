//! Render one repeated service with its exact Bot variables, not another Bot's environment.

use super::*;

impl PluginPackage {
    pub(in crate::plugin) fn bot_service(
        &self,
        name: &str,
        vars: &Vars,
        used: &mut UsedVars,
        langs: &Languages,
    ) -> Result<Service, PackageError> {
        let mut value = self
            .services
            .get(name)
            .expect("validated service name")
            .clone();
        strip_extensions(&mut value);
        interpolate_value(&mut value, vars, used)
            .map_err(|source| interpolate_error(langs, name.to_owned(), source))?;
        serde_path_to_error::deserialize(value)
            .map_err(|error| service_error(langs, name, pathed(&error)))
    }
}
