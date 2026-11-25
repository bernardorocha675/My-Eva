//! Atualização pela Central.
//!
//! - Só existe se houver endereço e chave pública em `plugins.updater` (tauri.conf.json).
//!   Sem isso, ou com "Atualizar sozinha" desligado, nada aqui usa a internet.
//! - O pedido lê um arquivo com versão, novidades e assinatura. Nada seu é enviado.
//! - O arquivo baixado só é instalado se a assinatura bater com a chave pública E for da
//!   mesma versão anunciada (requireSignedVersion).
//! - Nunca instala com a Eva ligada.
//! - No Windows e no AppImage troca sozinha; no .deb o sistema pede a senha, então só
//!   instala quando você clica.

use crate::{central, ponte};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

#[derive(Default)]
pub struct Atualizacao {
    achada: Mutex<Option<Update>>,
    /// Baixando ou instalando: a Eva não liga e a Central não fecha.
    trabalhando: AtomicBool,
}

impl Atualizacao {
    pub fn trabalhando(&self) -> bool {
        self.trabalhando.load(Ordering::SeqCst)
    }
}

struct Trabalho<'a>(&'a AtomicBool);
impl<'a> Trabalho<'a> {
    fn pegar(t: &'a AtomicBool) -> Option<Self> {
        (!t.swap(true, Ordering::SeqCst)).then_some(Self(t))
    }
}
impl Drop for Trabalho<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
pub struct EstadoAtualizacao {
    /// Há endereço e chave configurados?
    existe: bool,
    /// "Atualizar sozinha" ligado?
    ligada: bool,
    /// Instala sem pedir senha (Windows e AppImage)?
    sozinha: bool,
}

#[derive(Serialize)]
pub struct Nova {
    versao: String,
    notas: String,
}

fn existe(app: &AppHandle) -> bool {
    let cfg = app.config().plugins.0.get("updater");
    let tem = |campo: &str| {
        cfg.and_then(|c| c.get(campo)).is_some_and(|v| match v {
            serde_json::Value::String(s) => !s.is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            _ => false,
        })
    };
    tem("pubkey") && tem("endpoints")
}

/// A preferência é um arquivo vazio: existe = desligado.
fn marca_desligada() -> std::path::PathBuf {
    ponte::arquivo_ponte().with_file_name("atualizacao-desligada")
}

fn ligada() -> bool {
    !marca_desligada().exists()
}

/// Código curto que a interface entende.
fn codigo(e: Error) -> String {
    eprintln!("[eva] atualização: {e}");
    match e {
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        // Assinatura legítima, mas de outra versão: alguém pode estar forçando uma antiga.
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => "assinatura",
        Error::Reqwest(_) | Error::Network(_) => "rede",
        _ => "falhou",
    }
    .into()
}

#[tauri::command]
pub fn atualizacao_estado(app: AppHandle) -> EstadoAtualizacao {
    EstadoAtualizacao {
        existe: existe(&app),
        ligada: ligada(),
        sozinha: cfg!(windows) || tauri::utils::platform::bundle_type() == Some(BundleType::AppImage),
    }
}

#[tauri::command]
pub fn atualizacao_definir(ligar: bool) -> bool {
    let marca = marca_desligada();
    if ligar {
        let _ = std::fs::remove_file(&marca);
    } else {
        if let Some(p) = marca.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::write(&marca, "1");
    }
    ligada()
}

#[tauri::command]
pub async fn atualizacao_procurar(app: AppHandle, estado: State<'_, Atualizacao>) -> Result<Option<Nova>, String> {
    // Confere aqui também: desligada, nenhuma chamada da tela chega à internet.
    if !existe(&app) || !ligada() {
        return Ok(None);
    }
    let resultado = app
        .updater_builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(codigo)?
        .check()
        .await;
    let achada = match resultado {
        // O endereço respondeu, mas ainda não há versão publicada: não é falta de internet.
        Err(Error::ReleaseNotFound) => None,
        outro => outro.map_err(codigo)?,
    };
    let nova = achada.as_ref().map(|u| Nova {
        versao: u.version.clone(),
        notas: u.body.clone().unwrap_or_default(),
    });
    *estado.achada.lock().unwrap() = achada.map(|mut u| {
        // Os 10 s valem só pra procurar; baixar pode demorar mais.
        u.timeout = Some(Duration::from_secs(30 * 60));
        u
    });
    Ok(nova)
}

#[tauri::command]
pub async fn atualizacao_instalar(app: AppHandle, estado: State<'_, Atualizacao>) -> Result<(), String> {
    let Some(_trabalho) = Trabalho::pegar(&estado.trabalhando) else { return Err("ocupada".into()) };
    if central::ligada(&app) {
        return Err("ligada".into());
    }
    let Some(update) = estado.achada.lock().unwrap().clone() else { return Err("falhou".into()) };

    let (aviso, mut baixado, mut ultimo) = (app.clone(), 0u64, u64::MAX);
    let bytes = update
        .download(
            move |pedaco, total| {
                baixado += pedaco as u64;
                let pct = total.filter(|t| *t > 0).map(|t| (baixado * 100 / t).min(100)).unwrap_or(0);
                if pct != ultimo {
                    ultimo = pct;
                    let _ = aviso.emit_to("central", "atualizacao-progresso", pct);
                }
            },
            || {},
        )
        .await
        .map_err(codigo)?;
    // download() já conferiu a assinatura.
    let _ = app.emit_to("central", "atualizacao-instalando", ());
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|_| "falhou".to_string())?
        .map_err(codigo)?;
    app.request_restart();
    Ok(())
}
