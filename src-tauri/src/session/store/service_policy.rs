use super::*;

#[cfg(test)]
#[path = "service_policy_tests.rs"]
mod tests;

impl SessionStore {
    /// Detection persisted Unity as `lazy`, but the frontend started it on
    /// workspace focus, so it behaved as auto. `lazy` now really defers the
    /// service until its first tool call or SDK use; existing workspaces keep
    /// their previous behavior. User-disabled records are left untouched.
    pub(super) fn migrate_unity_service_auto_policy(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "UPDATE workspace_services
             SET activation_policy = 'auto'
             WHERE service_kind = 'unity' AND activation_policy = 'lazy'",
            [],
        )?;
        Ok(())
    }

    /// A lazy-mode service was bound after the run started; record the full
    /// binding set so the run history and context export include it. Only
    /// scoped runs carry bindings, so unscoped rows are left untouched.
    pub fn update_run_service_bindings(
        &self,
        run_id: &str,
        bindings: &[SessionRunServiceBinding],
    ) -> Result<(), String> {
        let json = serde_json::to_string(bindings)
            .map_err(|e| format!("Failed to serialize run service bindings: {}", e))?;
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE session_runs
             SET service_bindings_json = ?1
             WHERE run_id = ?2 AND checkout_id IS NOT NULL",
            params![json, run_id],
        )
        .map_err(|e| format!("Failed to update run service bindings: {}", e))?;
        Ok(())
    }
}
