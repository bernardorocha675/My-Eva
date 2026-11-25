//! Música tocando no Linux (Spotify, Firefox, Chrome, Brave, VLC...).
//!
//! Usa o MPRIS, o padrão do Linux para players de mídia (é o que o GNOME usa
//! no menu de mídia do topo da tela). Funciona pelo D-Bus da sessão, local.
//! Mesma ideia da versão do Windows: uma thread lê o estado 1x por segundo
//! e executa os comandos (tocar/pausar, próxima, anterior) que chegam pelo canal.
//!
//! Capa: só quando o player passa um arquivo local (file://) ou uma imagem
//! embutida (data:). Links da internet (o Spotify faz isso) ficam sem capa,
//! pra Eva continuar sem acessar a internet.

use base64::Engine;
use mpris::{PlaybackStatus, Player, PlayerFinder};
use serde::Serialize;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize, Default, PartialEq, Debug)]
pub struct MusicaStatus {
    pub ativo: bool,
    pub titulo: String,
    pub artista: String,
    /// Nome amigável do app (Spotify, Firefox, Chrome...).
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

/// Alguns players (o Spotify do Linux é o famoso) sempre dizem que a posição é 0.
/// Nesse caso a Eva conta o tempo sozinha desde que a faixa começou.
struct Relogio {
    faixa: String,
    base: f64,
    desde: Instant,
    tocando: bool,
}

pub fn iniciar(app: AppHandle, estado: Shared) -> mpsc::Sender<Acao> {
    let (tx, rx) = mpsc::channel::<Acao>();

    thread::spawn(move || {
        let mut finder: Option<PlayerFinder> = None;
        let mut ultima_faixa = String::new();
        let mut buscar_capa = 0u8;
        let mut relogio: Option<Relogio> = None;

        loop {
            let comando = match rx.recv_timeout(Duration::from_millis(1000)) {
                Ok(acao) => Some(acao),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };

            if finder.is_none() {
                finder = PlayerFinder::new().ok();
            }
            let Some(f) = finder.as_ref() else { continue };
            let players = f.find_all().unwrap_or_default();
            let sessao = escolher(&players);

            if let (Some(acao), Some(p)) = (comando, sessao) {
                let _ = match acao {
                    Acao::TocarPausar => p.play_pause(),
                    Acao::Proxima => p.next(),
                    Acao::Anterior => p.previous(),
                };
                thread::sleep(Duration::from_millis(250));
            }

            let mut status = sessao.map(ler).unwrap_or_default();
            corrigir_posicao(&mut status, &mut relogio);

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
                let capa = if status.ativo { sessao.and_then(ler_capa) } else { None };
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

/// Mesma prioridade da versão do Windows:
/// 1. Spotify tocando  2. qualquer um tocando  3. Spotify pausado  4. o primeiro que tiver música.
fn escolher(players: &[Player]) -> Option<&Player> {
    let tocando = |p: &&Player| matches!(p.get_playback_status(), Ok(PlaybackStatus::Playing));
    let eh_spotify = |p: &&Player| p.bus_name().to_lowercase().contains("spotify");
    let tem_musica = |p: &&Player| {
        p.get_metadata().ok().and_then(|m| m.title().map(|t| !t.is_empty())).unwrap_or(false)
    };

    players
        .iter()
        .find(|p| eh_spotify(p) && tocando(p))
        .or_else(|| players.iter().find(tocando))
        .or_else(|| players.iter().find(|p| eh_spotify(p) && tem_musica(p)))
        .or_else(|| players.iter().find(tem_musica))
}

fn ler(p: &Player) -> MusicaStatus {
    let Ok(meta) = p.get_metadata() else { return MusicaStatus::default() };
    let titulo = meta.title().unwrap_or_default().to_string();
    let artista = meta.artists().map(|a| a.join(", ")).unwrap_or_default();
    let tocando = matches!(p.get_playback_status(), Ok(PlaybackStatus::Playing));
    let duracao = meta.length().map(|d| d.as_secs_f64()).unwrap_or(0.0).round();
    let mut posicao = p.get_position().map(|d| d.as_secs_f64()).unwrap_or(0.0);
    if duracao > 0.0 {
        posicao = posicao.clamp(0.0, duracao);
    }

    MusicaStatus {
        ativo: !titulo.is_empty(),
        titulo,
        artista,
        app: nome_app(p),
        tocando,
        posicao: (posicao * 10.0).round() / 10.0,
        duracao,
    }
}

/// Se o player insiste em posição 0 enquanto toca, conta o tempo localmente.
fn corrigir_posicao(s: &mut MusicaStatus, relogio: &mut Option<Relogio>) {
    if !s.ativo {
        *relogio = None;
        return;
    }
    let faixa = format!("{}\u{1}{}", s.titulo, s.artista);
    let agora = Instant::now();
    let r = match relogio {
        Some(r) if r.faixa == faixa => r,
        _ => {
            *relogio = Some(Relogio { faixa, base: s.posicao, desde: agora, tocando: s.tocando });
            return;
        }
    };
    // Tempo contado pela Eva até agora.
    let contado = r.base + if r.tocando { agora.duration_since(r.desde).as_secs_f64() } else { 0.0 };
    if s.posicao > 0.0 {
        // O player informa a posição de verdade: só acompanha.
        *r = Relogio { faixa: r.faixa.clone(), base: s.posicao, desde: agora, tocando: s.tocando };
        return;
    }
    let mut estimado = contado;
    if s.duracao > 0.0 {
        estimado = estimado.min(s.duracao);
    }
    s.posicao = (estimado * 10.0).round() / 10.0;
    *r = Relogio { faixa: r.faixa.clone(), base: estimado, desde: agora, tocando: s.tocando };
}

fn ler_capa(p: &Player) -> Option<String> {
    let meta = p.get_metadata().ok()?;
    let url = meta.art_url()?.trim().to_string();
    if url.starts_with("data:image/") {
        return Some(url);
    }
    let caminho = url.strip_prefix("file://")?;
    let caminho = desescapar(caminho);
    let tamanho = std::fs::metadata(&caminho).ok()?.len();
    if tamanho == 0 || tamanho > 5_000_000 {
        return None;
    }
    let bytes = std::fs::read(&caminho).ok()?;
    let tipo = tipo_imagem(&bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{tipo};base64,{b64}"))
}

/// file:///home/x/Minha%20M%C3%BAsica/capa.jpg → /home/x/Minha Música/capa.jpg
fn desescapar(s: &str) -> String {
    let b = s.as_bytes();
    let mut saida = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                saida.push(v);
                i += 3;
                continue;
            }
        }
        saida.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).into_owned()
}

/// Descobre o tipo pela "assinatura" do arquivo (o MPRIS não informa).
fn tipo_imagem(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/png"
    }
}

fn nome_app(p: &Player) -> String {
    let l = format!("{} {}", p.bus_name(), p.identity()).to_lowercase();
    let conhecidos = [
        ("spotify", "Spotify"),
        ("brave", "Brave"),
        ("chromium", "Chromium"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("edge", "Edge"),
        ("firefox", "Firefox"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("rhythmbox", "Rhythmbox"),
        ("totem", "Vídeos"),
    ];
    for (chave, nome) in conhecidos {
        if l.contains(chave) {
            return nome.to_string();
        }
    }
    let id = p.identity();
    if id.is_empty() { p.bus_name_trimmed().to_string() } else { id.to_string() }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn desescapa_caminho_da_capa() {
        assert_eq!(desescapar("/home/x/Minha%20M%C3%BAsica/capa.jpg"), "/home/x/Minha Música/capa.jpg");
        assert_eq!(desescapar("/tmp/sem-escape.png"), "/tmp/sem-escape.png");
    }

    #[test]
    fn conta_tempo_quando_o_player_diz_zero() {
        let mut s = MusicaStatus { ativo: true, titulo: "a".into(), tocando: true, duracao: 200.0, ..Default::default() };
        let mut r = None;
        corrigir_posicao(&mut s, &mut r);
        r.as_mut().unwrap().desde -= Duration::from_secs(10);
        let mut s2 = s.clone();
        s2.posicao = 0.0;
        corrigir_posicao(&mut s2, &mut r);
        assert!((s2.posicao - 10.0).abs() < 0.5, "posição estimada: {}", s2.posicao);
    }
}
