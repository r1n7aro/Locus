//! Persisted connection identity. This small type is available in public builds
//! so they can preserve/export internal sessions without loading their transport.
use super::ssh::SshConfig;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeConnection {
    Ssh(SshConfig),
    Server { binding_id: String },
}

pub type ClaudeConnection = NativeConnection;

impl NativeConnection {
    pub fn server(binding_id: impl Into<String>) -> Result<Self, String> {
        let connection = Self::Server { binding_id: binding_id.into() };
        connection.validate()?;
        Ok(connection)
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Ssh(config) => config.validate(),
            Self::Server { binding_id } => uuid::Uuid::parse_str(binding_id)
                .map(|_| ()).map_err(|_| "Invalid Locus Server connection binding".into()),
        }
    }
    pub fn fingerprint(&self) -> String {
        match self {
            Self::Ssh(config) => config.fingerprint(),
            Self::Server { binding_id } => uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,
                format!("locus-server/{binding_id}").as_bytes()).to_string(),
        }
    }
    pub fn as_ssh(&self) -> Option<&SshConfig> {
        match self { Self::Ssh(config) => Some(config), Self::Server { .. } => None }
    }
    pub fn server_binding_id(&self) -> Option<&str> {
        match self { Self::Server { binding_id } => Some(binding_id), Self::Ssh(_) => None }
    }
    #[cfg(test)]
    pub fn as_ssh_mut(&mut self) -> Option<&mut SshConfig> {
        match self { Self::Ssh(config) => Some(config), Self::Server { .. } => None }
    }
}
impl From<SshConfig> for NativeConnection {
    fn from(value: SshConfig) -> Self { Self::Ssh(value) }
}
impl From<&SshConfig> for NativeConnection {
    fn from(value: &SshConfig) -> Self { Self::Ssh(value.clone()) }
}
impl Serialize for NativeConnection {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // Keep bytes/field order and therefore fingerprints of old SSH bindings.
            Self::Ssh(config) => config.serialize(serializer),
            Self::Server { binding_id } => {
                #[derive(Serialize)]
                #[serde(rename_all = "camelCase")]
                struct Reference<'a> { version: u32, transport: &'a str, binding_id: &'a str }
                Reference { version: 1, transport: "server", binding_id }.serialize(serializer)
            }
        }
    }
}
impl<'de> Deserialize<'de> for NativeConnection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("transport").is_some() {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Reference { version: u32, transport: String, binding_id: String }
            let reference: Reference = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            if reference.version != 1 || reference.transport != "server" {
                return Err(serde::de::Error::custom("Unsupported native connection version or transport"));
            }
            Self::server(reference.binding_id).map_err(serde::de::Error::custom)
        } else {
            serde_json::from_value(value).map(Self::Ssh).map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn old_ssh_bytes_and_fingerprint_are_unchanged() {
        let config: SshConfig = serde_json::from_value(json!({"target":"user@host","port":22})).unwrap();
        let bytes = serde_json::to_string(&config).unwrap();
        let connection: NativeConnection = serde_json::from_str(&bytes).unwrap();
        assert_eq!(serde_json::to_string(&connection).unwrap(), bytes);
        assert_eq!(connection.fingerprint(), config.fingerprint());
        assert_eq!(connection.as_ssh(), Some(&config));
    }
    #[test]
    fn server_reference_is_versioned_and_cannot_masquerade_as_ssh() {
        let id = uuid::Uuid::new_v4().to_string();
        let value = json!({"version":1,"transport":"server","bindingId":id});
        let connection: NativeConnection = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&connection).unwrap(), value);
        assert!(connection.as_ssh().is_none());
        for bad in [json!({"version":2,"transport":"server","bindingId":id}),
            json!({"version":1,"transport":"server","bindingId":id,"target":"host"}),
            json!({"version":1,"transport":"server","bindingId":"not-a-uuid"})] {
            assert!(serde_json::from_value::<NativeConnection>(bad).is_err());
        }
    }
}
