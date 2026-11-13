#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sftp_password_is_kept_out_of_the_url_and_private_key_is_supported() {
        let mut url = reqwest::Url::parse("sftp://example.test/file").unwrap();
        let fields = AuthFields {
            username: "alice".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let options = apply(&mut url, &fields, ndm_core::FtpSecurity::Auto).unwrap();
        assert_eq!(url.username(), "alice");
        assert_eq!(url.password(), None);
        assert!(!url.as_str().contains("secret"));
        assert!(matches!(
            options,
            ndm_core::TaskOptions::Sftp(ndm_core::SftpOptions {
                auth: Some(ndm_core::SftpAuth::Password(_)),
                ..
            })
        ));

        let fields = AuthFields {
            username: "alice".into(),
            key_path: "C:/keys/id_ed25519".into(),
            passphrase: "key-pass".into(),
            ..Default::default()
        };
        let options = apply(&mut url, &fields, ndm_core::FtpSecurity::Auto).unwrap();
        assert!(matches!(
            options,
            ndm_core::TaskOptions::Sftp(ndm_core::SftpOptions {
                auth: Some(ndm_core::SftpAuth::PrivateKey { .. }),
                ..
            })
        ));
    }

    #[test]
    fn ftps_implicit_mode_is_explicit_and_invalid_combinations_are_rejected() {
        let mut url = reqwest::Url::parse("ftps://example.test/file").unwrap();
        let fields = AuthFields {
            username: "alice".into(),
            password: "secret".into(),
            ..Default::default()
        };
        let options = apply(&mut url, &fields, ndm_core::FtpSecurity::ImplicitTls).unwrap();
        assert!(matches!(
            options,
            ndm_core::TaskOptions::Ftp(ndm_core::FtpOptions {
                security: ndm_core::FtpSecurity::ImplicitTls,
                ..
            })
        ));
        assert_eq!(url.password(), Some("secret"));
        let mut url = reqwest::Url::parse("https://example.test/file").unwrap();
        assert!(apply(&mut url, &fields, ndm_core::FtpSecurity::Auto).is_err());
    }
}
use ndm_core::{FtpOptions, FtpSecurity, SftpAuth, SftpOptions, TaskOptions};

#[derive(Default)]
pub struct AuthFields {
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub passphrase: String,
}

pub fn apply(
    url: &mut reqwest::Url,
    fields: &AuthFields,
    security: FtpSecurity,
) -> Result<TaskOptions, ()> {
    let supplied = !fields.username.is_empty()
        || !fields.password.is_empty()
        || !fields.key_path.is_empty()
        || !fields.passphrase.is_empty();
    if !supplied && security == FtpSecurity::Auto {
        return Ok(TaskOptions::Auto);
    }
    let scheme = url.scheme().to_owned();
    if !matches!(scheme.as_str(), "ftp" | "ftps" | "sftp")
        || (!fields.passphrase.is_empty() && fields.key_path.is_empty())
        || (!fields.password.is_empty() && !fields.key_path.is_empty())
    {
        return Err(());
    }
    if !fields.username.is_empty() {
        url.set_username(&fields.username).map_err(|_| ())?;
    }
    if (scheme == "sftp" || !fields.password.is_empty() || !fields.key_path.is_empty())
        && url.username().is_empty()
    {
        return Err(());
    }
    if scheme == "sftp" {
        if security != FtpSecurity::Auto {
            return Err(());
        }
        let auth = if !fields.key_path.is_empty() {
            url.set_password(None).map_err(|_| ())?;
            Some(SftpAuth::PrivateKey {
                path: fields.key_path.clone().into(),
                passphrase: (!fields.passphrase.is_empty()).then(|| fields.passphrase.clone()),
            })
        } else if !fields.password.is_empty() {
            url.set_password(None).map_err(|_| ())?;
            Some(SftpAuth::Password(fields.password.clone()))
        } else {
            None
        };
        Ok(TaskOptions::Sftp(SftpOptions {
            auth,
            ..Default::default()
        }))
    } else {
        if !fields.key_path.is_empty() || !fields.passphrase.is_empty() {
            return Err(());
        }
        if !fields.password.is_empty() {
            url.set_password(Some(&fields.password)).map_err(|_| ())?;
        }
        Ok(TaskOptions::Ftp(FtpOptions {
            security,
            ..Default::default()
        }))
    }
}
