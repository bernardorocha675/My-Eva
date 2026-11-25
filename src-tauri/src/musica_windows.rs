//! Música tocando no Windows (Spotify, YouTube no navegador, qualquer player).
//!
//! Usa a mesma API da janelinha de mídia do Windows (aquela que aparece quando
//! você aperta a tecla de volume): GlobalSystemMediaTransportControls.
//! Uma thread própria lê o estado 1x por segundo e também executa os comandos
//! (tocar/pausar, próxima, anterior) que chegam da interface por um canal.

use base64::Engine;
use serde::Serialize;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Sessao,
    GlobalSystemMediaTransportControlsSessionManager as Gerente,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as StatusPlayer,
};
use windows::Storage::Streams::DataReader;

#[derive(Clone, Serialize, Default, PartialEq, Debug)]
pub struct MusicaStatus {
    pub ativo: bool,
    pub titulo: String,
    pub artista: String,
    /// Nome amigável do app (Spotify, Chrome, Edge...).
    pub app: String,
    pub tocando: bool,
    /// Segundos desde o começo da faixa.
    pub posicao: f64,
    /// Duração em segundos (0 quando o player não informa).
    pub duracao: f64,
}

#[derive(Default)]
pub struct Estado {
    pub status: MusicaStatus,
    /// Capa como data URL (enviada separada porque é grande).
    pub capa: Option<String>,
}
pub type Shared = Arc<Mutex<Estado>>;

pub enum Acao {
    TocarPausar,
    Proxima,
    Anterior,
}
pub type Canal = Mutex<mpsc::Sender<Acao>>;

pub fn iniciar(app: AppHandle, estado: Shared) -> mpsc::Sender<Acao> {
    let (tx, rx) = mpsc::channel::<Acao>();

    thread::spawn(move || {
        // APIs WinRT precisam do COM inicializado nesta thread.
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            );
        }

        let mut gerente: Option<Gerente> = None;
        let mut ultima_faixa = String::new();
        // Alguns players trocam o nome antes da capa: tenta pegar a capa por alguns ciclos.
        let mut buscar_capa = 0u8;

        loop {
            // Espera até 1 s por um comando; sem comando, segue pra leitura.
            match rx.recv_timeout(Duration::from_millis(1000)) {
                Ok(acao) => {
                    if let Some(s) = gerente.as_ref().and_then(escolher_sessao) {
                        executar(&s, acao);
                    }
                    // Dá um tempinho pro player reagir antes de ler de novo.
                    thread::sleep(Duration::from_millis(250));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }

            if gerente.is_none() {
                gerente = Gerente::RequestAsync().and_then(|op| op.get()).ok();
            }
            let Some(g) = gerente.as_ref() else { continue };

            let sessao = escolher_sessao(g);
            let status = sessao
                .as_ref()
                .and_then(|s| ler(s).ok())
                .unwrap_or_default();

            let faixa = format!("{}\u{1}{}", status.titulo, status.artista);
            if faixa != ultima_faixa {
                ultima_faixa = faixa;
                buscar_capa = 3;
                #[cfg(debug_assertions)]
                eprintln!(
                    "[eva] música: '{}' — '{}' ({}) tocando={} {}s/{}s",
                    status.titulo, status.artista, status.app, status.tocando, status.posicao, status.duracao
                );
            }

            let mut capa_nova: Option<Option<String>> = None;
            if buscar_capa > 0 {
                buscar_capa -= 1;
                let capa = if status.ativo {
                    sessao.as_ref().and_then(|s| ler_capa(s).ok().flatten())
                } else {
                    None
                };
                capa_nova = Some(capa);
            }

            let mut e = estado.lock().unwrap();
            let mudou_status = e.status != status;
            if mudou_status {
                e.status = status.clone();
            }
            let mut emitir_capa = None;
            if let Some(c) = capa_nova {
                if e.capa != c {
                    e.capa = c.clone();
                    emitir_capa = Some(c);
                }
            }
            drop(e);

            if mudou_status {
                let _ = app.emit("musica", status);
            }
            if let Some(c) = emitir_capa {
                let _ = app.emit("musica-capa", c);
            }
        }
    });

    tx
}

/// Qual player mostrar quando há mais de um aberto, nesta ordem:
/// 1. Spotify tocando
/// 2. o player "atual" do Windows, se estiver tocando
/// 3. qualquer outro tocando
/// 4. Spotify pausado
/// 5. o player "atual" do Windows
/// Os comandos (pausar, pular...) usam a mesma escolha, então vão pro player que aparece na tela.
fn escolher_sessao(g: &Gerente) -> Option<Sessao> {
    let atual = g.GetCurrentSession().ok();
    let todas: Vec<Sessao> = g
        .GetSessions()
        .map(|lista| lista.into_iter().collect())
        .unwrap_or_default();

    let tocando = |s: &Sessao| {
        s.GetPlaybackInfo()
            .and_then(|i| i.PlaybackStatus())
            .map(|p| p == StatusPlayer::Playing)
            .unwrap_or(false)
    };
    let eh_spotify = |s: &Sessao| {
        s.SourceAppUserModelId()
            .map(|id| id.to_string().to_lowercase().contains("spotify"))
            .unwrap_or(false)
    };

    todas
        .iter()
        .find(|s| eh_spotify(s) && tocando(s))
        .cloned()
        .or_else(|| atual.clone().filter(|s| tocando(s)))
        .or_else(|| todas.iter().find(|s| tocando(s)).cloned())
        .or_else(|| todas.iter().find(|s| eh_spotify(s)).cloned())
        .or(atual)
}

fn executar(s: &Sessao, acao: Acao) {
    let _ = match acao {
        Acao::TocarPausar => s.TryTogglePlayPauseAsync().and_then(|op| op.get()),
        Acao::Proxima => s.TrySkipNextAsync().and_then(|op| op.get()),
        Acao::Anterior => s.TrySkipPreviousAsync().and_then(|op| op.get()),
    };
}

fn ler(s: &Sessao) -> windows::core::Result<MusicaStatus> {
    let props = s.TryGetMediaPropertiesAsync()?.get()?;
    let titulo = props.Title()?.to_string();
    let artista = props.Artist()?.to_string();

    let tocando = s.GetPlaybackInfo()?.PlaybackStatus()? == StatusPlayer::Playing;

    // Tempo: o player informa a posição na última atualização; se está tocando,
    // soma o tempo que passou desde então.
    let tl = s.GetTimelineProperties()?;
    let inicio = tl.StartTime()?.Duration as f64 / 1e7;
    let fim = tl.EndTime()?.Duration as f64 / 1e7;
    let mut posicao = tl.Position()?.Duration as f64 / 1e7 - inicio;
    if tocando {
        let atualizado = tl.LastUpdatedTime()?.UniversalTime;
        let passou = (agora_windows() - atualizado) as f64 / 1e7;
        if passou > 0.0 && passou < 3600.0 {
            posicao += passou;
        }
    }
    let duracao = (fim - inicio).max(0.0);
    if duracao > 0.0 {
        posicao = posicao.clamp(0.0, duracao);
    }

    Ok(MusicaStatus {
        ativo: !titulo.is_empty(),
        titulo,
        artista,
        app: nome_app(&s.SourceAppUserModelId()?.to_string()),
        tocando,
        posicao: (posicao * 10.0).round() / 10.0,
        duracao: duracao.round(),
    })
}

fn ler_capa(s: &Sessao) -> windows::core::Result<Option<String>> {
    let props = s.TryGetMediaPropertiesAsync()?.get()?;
    let Ok(miniatura) = props.Thumbnail() else { return Ok(None) };
    let stream = miniatura.OpenReadAsync()?.get()?;
    let tamanho = stream.Size()? as u32;
    if tamanho == 0 || tamanho > 5_000_000 {
        return Ok(None);
    }
    let tipo = stream.ContentType()?.to_string();
    let leitor = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    leitor.LoadAsync(tamanho)?.get()?;
    let mut bytes = vec![0u8; tamanho as usize];
    leitor.ReadBytes(&mut bytes)?;
    let tipo = if tipo.is_empty() { "image/png".to_string() } else { tipo };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:{tipo};base64,{b64}")))
}

/// Agora no formato de data do Windows (intervalos de 100 ns desde 1601).
fn agora_windows() -> i64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_nanos() / 100) as i64 + 116_444_736_000_000_000
}

fn nome_app(id: &str) -> String {
    let l = id.to_lowercase();
    let conhecidos = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("brave", "Brave"),
        ("firefox", "Firefox"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("zune", "Media Player"),
        ("mediaplayer", "Media Player"),
    ];
    for (chave, nome) in conhecidos {
        if l.contains(chave) {
            return nome.to_string();
        }
    }
    id.rsplit(['!', '\\']).next().unwrap_or(id).trim_end_matches(".exe").to_string()
}
