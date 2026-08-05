use anyhow::{Context, Result};
use localsend::crypto::cert::{fingerprint_from_cert_der, generate_self_signed};
use std::{fs, path::Path};

#[derive(Clone, Debug)]
pub struct IdentityMaterial {
    pub certificate_pem: String,
    pub private_key_pem: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct DeviceIdentity {
    pub alias: String,
    pub port: u16,
    pub material: IdentityMaterial,
}

impl DeviceIdentity {
    pub fn load_or_generate(dir: &Path, alias: String, port: u16) -> Result<Self> {
        let path = dir.join("identity.pem");
        match fs::read_to_string(&path) {
            Ok(text) => Self::from_pem(&text, alias, port)
                .with_context(|| format!("invalid identity file {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let identity = Self::generate(alias, port)?;
                write_private_file(
                    &path,
                    &identity.material.certificate_pem,
                    &identity.material.private_key_pem,
                )?;
                Ok(identity)
            }
            Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    fn from_pem(text: &str, alias: String, port: u16) -> Result<Self> {
        let blocks = pem::parse_many(text)?;
        let cert = blocks
            .iter()
            .find(|block| block.tag() == "CERTIFICATE")
            .context("missing certificate block")?;
        let key = blocks
            .iter()
            .find(|block| block.tag().ends_with("PRIVATE KEY"))
            .context("missing private key block")?;
        let private_key_pem = pem::encode(key);
        rcgen::KeyPair::from_pem(&private_key_pem).context("unusable private key")?;
        Ok(Self {
            alias,
            port,
            material: IdentityMaterial {
                certificate_pem: pem::encode(cert),
                private_key_pem,
                fingerprint: fingerprint_from_cert_der(cert.contents()),
            },
        })
    }

    fn generate(alias: String, port: u16) -> Result<Self> {
        let cert = generate_self_signed()?;
        Ok(Self {
            alias,
            port,
            material: IdentityMaterial {
                certificate_pem: cert.certificate_pem,
                private_key_pem: cert.private_key_pem,
                fingerprint: cert.fingerprint,
            },
        })
    }

    pub fn tls_config(&self) -> localsend::http::server::TlsConfig {
        localsend::http::server::TlsConfig {
            cert: self.material.certificate_pem.clone(),
            private_key: self.material.private_key_pem.clone(),
        }
    }
}

fn write_private_file(path: &Path, certificate_pem: &str, private_key_pem: &str) -> Result<()> {
    let contents = format!("{certificate_pem}{private_key_pem}");
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
    }
    #[cfg(not(unix))]
    fs::write(path, contents)?;
    Ok(())
}
