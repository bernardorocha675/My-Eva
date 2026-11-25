# My Eva

Companheira de mesa pessoal pro Windows e pro Linux (Ubuntu/GNOME), em PT-BR. Ela fica no topo da tela, no centro, e mostra o que está rolando no PC: a bateria do controle, a música e os avisos do Claude Code. Dá pra responder ao Claude direto por ela.

Ela **não é uma IA**. É um programa leve que roda localmente e não manda nada seu pra internet. A atualização pela Central está pronta, mas **desativada**: só passa a consultar a internet quando houver endereço e chave pública em `plugins.updater` (`tauri.conf.json`), e dá pra desligar em Ajustes.

**Versão atual: 1.1.1**

---

## O que ela faz

| Seção | O que mostra / faz |
|---|---|
| **Geral** | Relógio e um "chip" de resumo para cada seção |
| **Controle** | Bateria do DualShock 4 (Bluetooth ou USB), se está carregando e o tipo de conexão |
| **Música** | Capa grande, título, artista e progresso. Botões de tocar/pausar, próxima e anterior. Prioriza o Spotify e funciona com qualquer player que apareça nos controles de mídia do Windows (Brave, YouTube etc.) |
| **Claude** | Avisos do Claude Code: "terminei", "tô esperando", pedidos de permissão e perguntas. Dá pra responder, permitir, negar ou escolher opções pela Eva |

### Jeito dela
- **Entrada:** desce do topo, revira os olhos no sentido horário enquanto o nome **EVA** em pixel aparece com um acorde suave. Depois debocha, pisca e abre o painel.
- **Painel:** uma seção por vez, deslizando para os lados.
- **Some sozinha** depois de 5 s e vira uma alcinha fina (grafite) no topo da tela. Volta passando o mouse no topo ou apertando **INS**.
- **Expressões:** neutra, deboche, séria, curtindo (com música tocando) e piscadinha. Se você pausa a música e ela estava curtindo, ela fica séria.
- **Olhos:** acompanham o mouse.
- No Linux, a alcinha só brilha quando o Claude está esperando (o brilho contínuo gastava ~25% de um núcleo).
- **Brilho apressado** na alcinha quando o Claude está esperando você (cor muda por tipo de aviso).
- **Paleta:** Noite (escura) com alcinha Grafite.
- **Mascote:** original, de inspiração anime: cabelo roxo, óculos e queixo em "C fino suave".

---

## Como usar

- **Instalar:** rode `src-tauri/target/release/bundle/nsis/My Eva_<versão>_x64-setup.exe` (gerado no build). Ele instala só pro seu usuário, em `%LOCALAPPDATA%\My Eva\`, sem pedir administrador, e cria o atalho **My Eva** no menu Iniciar.
- **Iniciar com o Windows:** já vem ligado na primeira vez. Para desligar ou ligar, use o menu do ícone "E" na bandeja (perto do relógio) → **Iniciar com o Windows**.
- **Fechar:** ícone na bandeja → **Sair da Eva**.
- **Mostrar/esconder:** tecla **INS** ou mouse no topo da tela.

---

---

## Linux (Ubuntu / GNOME)

Mesmo código do Windows; só a bateria do controle, a música e alguns detalhes da janela mudam por sistema.

### Instalar
```bash
sudo apt install "./src-tauri/target/release/bundle/deb/My Eva_<versão>_amd64.deb"
```
Abre pelo menu de apps (**My Eva**). Também sai um AppImage (`bundle/appimage/`), que roda sem instalar.

### O que muda em relação ao Windows
| Coisa | No Linux |
|---|---|
| **Música** | Pelo **MPRIS** (o mesmo do menu de mídia do GNOME): Spotify, Firefox, Chrome, Brave, VLC... Botões funcionam. |
| **Capa** | Só quando o player passa um arquivo local (Firefox, Chrome, VLC costumam passar). **O Spotify passa um link da internet, então fica sem capa**, pra Eva continuar sem acessar a internet. |
| **Tempo da música** | O Spotify do Linux sempre diz "posição 0"; a Eva conta o tempo sozinha nesse caso. |
| **Controle (DS4)** | Lê a bateria que o próprio kernel já mostra em `/sys/class/power_supply/`. Não precisa de permissão nem regra udev. |
| **Janela** | Roda pelo **XWayland** (modo de compatibilidade que já vem no Ubuntu), porque o Wayland puro não deixa o app escolher a posição. Fica logo **abaixo da barra do topo** do GNOME (a barra fica por cima de qualquer janela). |
| **Cliques** | A janela é recortada no formato da Eva; o resto deixa o clique passar. |
| **Olhos** | No GNOME com Wayland, seguem o mouse só quando ele está em cima da Eva (o sistema não deixa ver o mouse fora dela). |
| **Tecla INS** | O GNOME com Wayland **não deixa app nenhum ter tecla global**. Solução: um atalho do próprio GNOME que roda `my-eva --alternar` (veja abaixo). |
| **Bandeja** | O ícone "E" aparece pela extensão de indicadores que já vem ligada no Ubuntu. |
| **Iniciar com o sistema** | Atalho em `~/.config/autostart/`. Liga sozinho na primeira vez; desliga pelo menu da bandeja. |
| **Ponte** | `~/.config/com.myeva.app/ponte.json`, só o seu usuário lê (pasta 700, arquivo 600). Token do gerador criptográfico do sistema. |

### Tecla INS no GNOME (atalho do sistema)
Em **Configurações → Teclado → Atalhos de teclado → Ver e personalizar → Atalhos personalizados → +**:
- Nome: `Eva`
- Comando: `my-eva --alternar`
- Atalho: aperte **Insert**

Ou pelo terminal (isso **substitui** a lista de atalhos personalizados; só use se você não tiver outros):
```bash
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/eva/']"
```
```bash
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/eva/ name 'Eva'
```
```bash
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/eva/ command 'my-eva --alternar'
```
```bash
gsettings set org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/eva/ binding 'Insert'
```
`my-eva --alternar` mostra/esconde a Eva ligada; com ela desligada, abre a Central.

### Compilar no Linux
```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev build-essential curl
```
```bash
npm install && npm run build
```

### Wayland puro (experimental)
Pra testar sem XWayland: `GDK_BACKEND=wayland my-eva`. A Eva abre, mas o GNOME decide onde ela fica e "sempre por cima" não é garantido.

---

## Tecnologia

- **Tauri v2** (Rust + HTML/CSS/JS puro, sem framework), janela transparente, sempre no topo, sem borda, 700×560.
- Os cliques passam direto pela janela onde a Eva não está visível (o Rust vigia o cursor e liga/desliga `set_ignore_cursor_events`).
- **hidapi** para a bateria do DS4 (Windows); `/sys/class/power_supply` no Linux.
- **SMTC do Windows** (crate `windows`) para a música; **MPRIS** (crate `mpris`) no Linux. A capa vira imagem local em base64.
- **Plugins:** `global-shortcut` (INS) e `autostart` (registro `HKCU\...\Run`).
- **Sons** sintetizados com WebAudio (nada de arquivo de áudio).
- **Instalador:** NSIS, por usuário (Windows); `.deb` e AppImage (Linux, em `tauri.linux.conf.json`).

### Estrutura

```
src/                 interface (index.html, style.css, main.js), Central (central.*) e novidades.js
src-tauri/src/
  main.rs            janela, bandeja, tecla INS, iniciar com o sistema, cursor
  ds4_windows.rs     bateria do DualShock 4 (HID)
  ds4_linux.rs       bateria do DualShock 4 (/sys/class/power_supply)
  musica_windows.rs  música pelos controles de mídia do Windows
  musica_linux.rs    música pelo MPRIS (+ testes)
  central.rs         Central: liga e desliga a Eva
  atualizacao.rs     atualização pela Central (parada até ter endereço e chave)
  ponte.rs           "interfone" com o Claude Code (+ testes)
```

### Comandos

```powershell
npm run dev                                    # versão de testes
npm run build                                  # versão final + instalador
cd src-tauri; cargo test --bin my-eva ponte::  # testes da ponte
cd src-tauri; cargo test                       # todos (no Linux inclui os da música)
```

---

## Integração com o Claude Code

A Eva abre uma **ponte HTTP só em `127.0.0.1`**, numa porta aleatória, com uma senha (token) nova a cada vez que abre. Porta e senha ficam em `%APPDATA%\com.myeva.app\ponte.json`. Um hook do Claude Code lê esse arquivo, manda o pedido e devolve a resposta da Eva. Os scripts dos hooks não fazem parte deste repositório.

| Hook | Evento da Eva | O que acontece |
|---|---|---|
| `PermissionRequest` (todas as ferramentas) | `permissao` | Tela de aprovação: permitir uma vez, sempre permitir a regra, negar com motivo, ou deixar pro próprio Claude |
| `PreToolUse` (AskUserQuestion) | `pergunta` | Escolher as opções pela Eva |
| `Stop` | `terminei` | "Terminei a tarefa"; dá pra responder e o Claude continua |
| `Notification` (idle_prompt) | `aviso` | Só avisa que ele está esperando |

Se a Eva estiver fechada ou desligada na Central, o hook sai em ~1 s sem resposta e o Claude Code segue o fluxo normal dele.

### Regras de segurança da aprovação (desde a 1.0.1/1.0.2)
- O **resumo** é montado pela Eva a partir do pedido real. A descrição escrita pelo Claude aparece só como comentário ("O Claude descreve como").
- O **comando** e **todos os argumentos** do pedido aparecem na tela (para ferramentas MCP: "Tudo o que vai ser enviado").
- **"Sempre permitir"** só grava regra exata:
  - Terminal (Bash/PowerShell): o comando inteiro, igualzinho. Sugestões com curinga (`git:*`) são ignoradas.
  - Outras ferramentas: só sugestões **sem `*`**, da mesma ferramenta, gravadas só pra você ou só na sessão.
  - A regra aparece **inteira** antes de confirmar; regra com mais de 300 caracteres ou pedido grande demais não oferece "Sempre".
  - Pedido de risco alto não oferece "Sempre".
- **Risco** (baixo/médio/alto) é uma estimativa por palavras-chave e a tela diz isso.
- Trava de 700 ms contra clique duplo quando o próximo pedido da fila aparece.
- A ponte confere a senha **antes** de abrir thread e atende no máximo 16 recados ao mesmo tempo.
- Lê só os últimos 512 KB do histórico do Claude para o "Por quê".

---

## Histórico de versões

| Versão | O que mudou |
|---|---|
| 1.0.0 | Instalador, iniciar com o Windows, mais leve quando escondida, entrada nova (revirar de olhos + nome em pixel + acorde), tela Geral |
| 1.0.1 | Correções de uma revisão de segurança: resumo montado pela Eva, comando visível, "Sempre" mais restrito, ponte limitada |
| 1.0.2 | "Sempre permitir" sem curingas e com prévia inteira, todos os argumentos na tela, risco mais honesto, trava de clique duplo, leitura só do fim do histórico |
| 1.1.0 | **Versão Linux** (Ubuntu/GNOME): música pelo MPRIS, DS4 pelo kernel, `.deb`/AppImage, `my-eva --alternar`. Risco reconhece comandos do Linux (`sudo`, `dd`, `mkfs`...). Correção (vale pro Windows também): a marca de "iniciar com o sistema já configurado" podia não ser gravada na primeira abertura, então o início automático podia religar uma vez, na segunda |
| 1.1.1 | **Central**: o programa abre numa janela que liga e desliga a Eva, com novidades, versões e ajustes; pergunta antes de desligar com pedido do Claude esperando. Uma Eva só também no Windows. No Linux, some certo no Wayland e abre no centro |

---

## Licença

Todos os direitos reservados. Veja [LICENSE](LICENSE).
