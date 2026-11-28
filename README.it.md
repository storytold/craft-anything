<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<p align="center">
  <img src="assets/app-icon/hicolor/256x256/apps/ai.storyteller.craft-anything.png" width="128" height="128" alt="Icona di Craft Anything: un corvo inciso su una tessera blu">
</p>

<h1 align="center">Craft Anything</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README.ja.md">日本語</a> · <a href="README.zh.md">简体中文</a> · <a href="README.zh-hant.md">繁體中文</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <b>Italiano</b> · <a href="README.vi.md">Tiếng Việt</a> · <a href="README.pt.md">Português</a> · <a href="README.th.md">ไทย</a> · <a href="README.tr.md">Türkçe</a> · <a href="README.he.md">עברית</a>
</p>

<p align="center">
  <b>Scopri come funziona qualsiasi sito web, app o gioco, poi lascia che l'AI crei la tua versione: un engine open source, clean room, scritto interamente in Rust.</b><br>
  Indica cosa vuoi esaminare e cosa vuoi ottenere: studierà l'originale con veri tool di ispezione,<br>
  affiderà a un coding agent la creazione di un nuovo progetto, confronterà il risultato con l'originale e lo perfezionerà.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="macOS · Windows · Linux · FreeBSD · Web" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-2d81ff?style=flat-square">
  <img alt="Licenza: MIT oppure Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Stato: prima versione alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
  <br>
  <a href="https://sonarcloud.io/project/overview?id=storytold_craft-anything"><img alt="Analizzato con SonarCloud" src="https://img.shields.io/badge/Scanned_by-SonarCloud-4b9fd5?style=flat-square&logo=sonarqubecloud&logoColor=white"></a>
  <a href="https://coderabbit.ai"><img alt="Revisionato con CodeRabbit" src="https://img.shields.io/coderabbit/prs/github/storytold/craft-anything?style=flat-square&label=Reviewed%20by%20CodeRabbit&logo=coderabbit"></a>
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Unisciti alla community ArtCraft su Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="#download"><b>Scarica Craft Anything</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">Tutte le Crafting Apps</a>
</p>

<br>

<p align="center">
  <img src="docs/images/project-website.png" alt="Un progetto di sito web in Craft Anything: il check elenca le differenze ancora presenti e la somiglianza di ogni pagina, il builder indica che la sua versione è pronta, una scheda chiede il consenso prima di eseguire un comando e il piano mostra due fasi completate su quattro" width="100%">
  <br>
  <sub>Un progetto di sito web: ogni check indica quanto il risultato somiglia all'originale, approvi le operazioni fuori dalle regole e il piano mostra cosa resta da fare.</sub>
</p>

> [!NOTE]
> **ArtCraft è una community di artisti di ogni ambito.** Pittori, fotografi,
> registi, illustratori, designer, animatori, appassionati e persone che hanno preso in mano una matita
> la settimana scorsa. Se crei qualcosa, sei dei nostri. **[Passa a salutarci su Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#come-funziona">Come funziona</a> ·
  <a href="#due-modi-per-usarlo">Due modi per usarlo</a> ·
  <a href="#ai">AI</a> ·
  <a href="#pensato-per-gli-agent">Agent</a> ·
  <a href="#sotto-il-cofano">Sotto il cofano</a> ·
  <a href="#per-iniziare">Per iniziare</a> ·
  <a href="#status">Status</a> ·
  <a href="#download">Download</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3>🔍 Studia qualsiasi cosa</h3>
      Siti web, desktop app, app mobile, giochi e file: veri tool di ispezione scoprono come sono costruiti e cosa fanno, con le evidence a sostegno di ogni risposta.
    </td>
    <td width="25%" valign="top">
      <h3>🛠️ Crea la tua versione</h3>
      Descrivi cosa vuoi ("Ricrea questo sito web", "Ricrea il gameplay di questo gioco in Rust") e un coding agent creerà un nuovo progetto nello stack che richiedi.
    </td>
    <td width="25%" valign="top">
      <h3>✅ Verificato, senza tirare a indovinare</h3>
      Ogni build viene confrontata con l'originale (pagine a tre larghezze dello schermo, il self-test di un gioco, i run di un'app) e perfezionata finché non gli somiglia abbastanza.
    </td>
    <td width="25%" valign="top">
      <h3>🤖 Pronto per gli agent</h3>
      320 tool, ciascuno disponibile come comando CLI e tool MCP, così qualsiasi agent può usare lo stesso engine dell'app. Per ispezionare non serve l'AI.
    </td>
  </tr>
</table>

<br>

## Come funziona

Indica a Craft Anything cosa esaminare, descrivi cosa vuoi e premi Start. Il lavoro si svolge in quattro fasi,
con domande in parole semplici quando una decisione spetta a te:

1. **Study (Studio):** ispeziona l'originale: le pagine a ogni larghezza, il codice, i file, il comportamento a runtime.
2. **Build (Creazione):** un coding agent crea un nuovo progetto nel linguaggio, framework o engine
   indicato nella richiesta, oppure sceglie un'opzione predefinita adatta.
3. **Check (Verifica):** confronta il risultato con l'originale ed elenca le differenze ancora presenti.
4. **Ready (Pronto):** apri, gioca o visualizza l'anteprima del risultato, oppure chiedi modifiche per perfezionarlo.

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/images/make.png" alt="La schermata Make: scegli un sito web, un'app o un gioco da ricreare, inserisci l'indirizzo, scegli Clean room o Deep, descrivi cosa vuoi e premi Start" width="100%"><br>
      <sub><b>Make:</b> scegli cosa ricreare, descrivi cosa vuoi e premi Start.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/images/project-game.png" alt="Un progetto completato del gioco del serpente, con tutte le fasi concluse e il pulsante Play the game" width="100%"><br>
      <sub><b>Ready:</b> gioca oppure chiedi modifiche nel riquadro in basso.</sub>
    </td>
  </tr>
</table>

## Due modi per usarlo

- **Simple** (predefinito): "What do you want to make?" (Cosa vuoi creare?). Scegli un sito web, un'app o un gioco,
  descrivilo e premi Start. La pagina del progetto mostra le quattro fasi (Study → Build → Check → Ready), cosa sta
  facendo l'agent in parole semplici, le question card e le approval card, quanto il risultato somiglia
  all'originale e i pulsanti per aprire il risultato o la sua cartella. Chiedi modifiche nel riquadro in basso.
  "Just look inside" apre un target per porre domande senza creare nulla (in modalità Deep).
- **Advanced**: l'ambiente di lavoro completo, con tutti i tool (Ctrl/Cmd+K), functions, strings, pseudo-C,
  disassembly, hex, call graph, evidence, unknowns, job, log e console dell'agent.

Entrambe le modalità usano lo stesso engine.

## Clean room o Deep

- **Clean room** (predefinita): non viene letto nulla all'interno dell'originale: né il codice, né i file, né il
  page source. Craft Anything lavora su ciò che puoi vedere: le immagini che aggiungi, il sito web come
  lo vede un visitatore (screenshot, testo visibile, misure del layout, colori, font, spaziature e menu aperti)
  e, quando lo consenti, un'immagine della finestra dell'app. I tool che leggerebbero l'interno vengono
  bloccati e spiegano come attivare la modalità Deep.
- **Deep**: vengono usati tutti i tool, per il tuo uso privato. Il codice e i file di un programma vengono
  letti con gli engine integrati (e con Ghidra, se installato); viene letto il JavaScript di un sito web e osservata
  la sua pagina iniziale. L'agent che crea il progetto può usare tutte queste informazioni.

Clean room è una regola su ciò che Craft Anything offre o rifiuta. Non è una sandbox a livello di OS:
non isola il tuo computer. Un coding agent che esegui agisce comunque con i tuoi permessi; ciò che
può leggere, scrivere o eseguire dipende dalle sue autorizzazioni. I tool propri di un agent che
Craft Anything può bloccare variano da agent ad agent; per i tool di Claude Code nelle chat e nelle domande,
consulta [`docs/claude-code.md`](docs/claude-code.md).

Puoi cambiare modalità dalla barra del titolo o da Settings, con `settings_set deep_mode true`, oppure per una
singola esecuzione da CLI con `--deep` (o `--clean-room`).

<p align="center">
  <img src="docs/images/workbench.png" alt="L'ambiente Advanced: un elenco di functions, lo pseudo-C della function selezionata, l'agent che risponde a una domanda chiamando un tool e l'evidence ledger" width="100%">
  <br>
  <sub><b>Advanced:</b> functions, pseudo-C, l'agent e le evidence a sostegno di ogni risposta.</sub>
</p>

## AI

Per le build serve un coding agent sul tuo computer. **Claude Code** è il primo supportato:
Craft Anything esegue il tuo programma `claude` originale e non modificato, che usa il tuo accesso personale;
Craft Anything non vede né gestisce mai il tuo account. Il suo utilizzo rientra nel tuo contratto con Anthropic.
Consulta [docs/claude-code.md](docs/claude-code.md) per sapere come funziona e perché. Esiste un adapter per
Codex, ma non è ancora stato verificato su un'installazione reale. **GitHub Copilot** può usare la tua Copilot CLI
installata tramite ACP: accedi separatamente con `copilot login`. **Grok Build** può usare il programma `grok`
installato tramite ACP (`grok agent stdio`): accedi separatamente con `grok login`.
Consulta [docs/grok.md](docs/grok.md). **Kimi Code CLI** può usare il programma `kimi` installato tramite
ACP (`kimi acp`): accedi separatamente con `kimi login`; non è ancora stato eseguito in condizioni reali.
Puoi attivare ACP per ciascun backend anche con altri agent che lo
supportano (Gemini CLI, Qwen Code, opencode, goose, Hermes, Maki, Devin); tra questi, Maki e Devin sono stati
eseguiti in condizioni reali (Devin si autentica con la chiave salvata nel backend: il suo accesso CLI separato
non vale per ACP). Anthropic, OpenAI, OpenRouter, xAI (Grok, quando è impostata `XAI_API_KEY`) e i server locali
compatibili con OpenAI sono disponibili per domande e analisi (Settings → AI).

## Pensato per gli agent

Ogni tool è anche un subcommand CLI e un tool MCP, così un agent può studiare un target
o ricrearlo interamente senza passare dall'app:

```sh
craft-anything-cli commands                                   # tutti i tool, per toolset
craft-anything-cli mcp                                        # li espone a qualsiasi client MCP
craft-anything-cli rebuild https://example.com "Recreate this website"
```

I risultati sono record di evidence con la relativa confidence, la coverage e le limitazioni;
quelli più grandi vengono restituiti prima come summary e si consultano per pagine con
`core_evidence_browse`. `craft-anything-cli setup` collega Craft Anything alle app dei coding agent.

## Sotto il cofano

Un engine di reverse engineering scritto interamente in Rust, con 320 tool (300 nell'engine principale,
gli altri nei layer di AI e setup) per programmi nativi, .NET, Java, Android, Apple,
Electron/JavaScript, siti web, firmware, packet capture, database e file di giochi.
I suoi decoder, il disassembler e il decompiler integrati funzionano senza programmi esterni;
la modalità Deep li usa tutti, insieme a Ghidra quando è installato. L'engine alimenta la desktop app (egui),
la CLI e il server MCP, e alcune sue parti funzionano nel browser come WebAssembly. L'elenco completo è in
[docs/tools.md](docs/tools.md).

## Per iniziare

Richiede **Rust 1.97 o successivo** (necessario per una dipendenza; esegui `rustup update`). Su Debian o Ubuntu,
la desktop app richiede anche:

```sh
sudo apt-get install build-essential libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev
```

Poi:

```sh
git clone https://github.com/storytold/craft-anything && cd craft-anything
cargo run --release -p craft-anything                # la desktop app
cargo install --path apps/craft-anything-cli         # la CLI, come `craft-anything-cli`
craft-anything-cli doctor                            # controlla installazione, coding agent ed engine
```

Per le build serve un coding agent con l'accesso effettuato sul computer (vedi [AI](#ai));
`doctor` indica cosa manca. Puoi ricreare un progetto dalla riga di comando (le domande vengono poste nel terminale):

```sh
craft-anything-cli rebuild https://example.com "Recreate this website"
craft-anything-cli rebuild - "A snake game in Rust" --rounds 1
craft-anything-cli rebuild https://example.com "Make the header blue" --continue-in "<project folder>"
craft-anything-cli rebuild - "Add a contact form" --project-folder "<your folder>" --existing edit
```

I nuovi progetti vengono salvati in `Documents/Craft Anything` (modificabile in Settings → General). Ogni cartella
di progetto contiene `craft.json` (richiesta, target, stato e cronologia), `capture/` (ciò che è stato
registrato dall'originale), `app/` (il progetto generato), `checks/` (ogni confronto con l'originale),
`investigation.cany/` (evidence) e `inbox/` (domande aperte).

Un progetto può partire anche da una cartella che scegli tu (Choose a folder… sotto Start, oppure
`--project-folder`). Se contiene già dei file, puoi indicare cosa rappresentano: l'app stessa, che l'agent
modificherà direttamente, oppure materiale di partenza, con la nuova app creata in `app/` accanto ai file
esistenti (Settings → General può ricordare la risposta). Settings → General elenca anche i progetti che si trovano
in altre cartelle; rimuoverne uno lo toglie soltanto dall'elenco.

## Status

> [!IMPORTANT]
> **Status:** Craft Anything è nelle prime fasi alpha. Cosa funziona oggi, cosa è parziale e cosa arriverà:
>
> - **Funziona oggi:** la desktop app (Simple e Advanced), la CLI e il server MCP; il flusso
>   Study → Build → Check → Ready per siti web, app e giochi con Claude Code come builder;
>   check dei siti web a tre larghezze dello schermo; rebuild di giochi in Unity, Unreal o Godot;
>   modalità Deep con Ghidra; immagini e modelli 3D generati (opt-in, con la tua chiave fal.ai);
>   plugin (server MCP e componenti WASM nelle build con l'opzione `wasm-plugins`).
> - **Parziale:** build con chiavi API e altri coding agent (implementate, ma non
>   ancora verificate in un utilizzo reale), check del comportamento di app e giochi, alcuni tool
>   dell'engine testati soltanto con stand-in.
> - **Rilasciato:** sulla [pagina delle release](https://github.com/storytold/craft-anything/releases) sono
>   pubblicate versioni con installer per Windows, macOS, Linux e FreeBSD; sono ancora alpha.
> - **In arrivo:** il managed tier.
>
> [docs/STATUS.md](docs/STATUS.md) descrive i dettagli e come è stata verificata ogni parte,
> [docs/tool-status.md](docs/tool-status.md) contiene la checklist per ciascun tool e
> [docs/ROADMAP.md](docs/ROADMAP.md) il piano, punto per punto. Apri una issue oppure segnala cosa non
> funziona su [Discord](https://discord.gg/artcraft).

**Uso responsabile.** Usa Craft Anything su software di tua proprietà o che sei autorizzato a esaminare.
Non aggira DRM, accessi o sistemi anti-cheat. I progetti ricreati riproducono layout, stile e comportamento
in base alle evidence; l'agent riceve l'istruzione di non copiare loghi, foto o testi, a meno che tu non
confermi di esserne il proprietario o di avere il permesso.

**L'installazione delle dipendenze esegue codice.** Quando un agent installa le dipendenze di un progetto,
i comandi vengono eseguiti sul tuo computer e i package possono eseguire codice durante l'installazione.
Per impostazione predefinita, npm esegue gli install script di ciascun package; anche i Rust build script e i source package
Python eseguono il proprio setup code.
Craft Anything chiede conferma prima di eseguire un comando di un tipo non ancora usato nel progetto (`npm`, `cargo`…).
Scegliere "always allow" per `npm` consente di eseguire tutti i successivi comandi npm senza altre conferme:
fallo soltanto per un progetto delle cui dipendenze ti fidi. Per installare package npm senza i loro script,
esegui `npm install --ignore-scripts`.

## Download

Su GitHub trovi i download nell'[ultima release](https://github.com/storytold/craft-anything/releases/latest);
[tutte le release](https://github.com/storytold/craft-anything/releases) raccoglie le versioni precedenti e le relative note.
Le tabelle elencano formati e nomi dei file: controlla gli asset della release scelta per sapere quali build sono disponibili.
`<ver>` nei nomi dei file indica il numero di versione e `SHA256SUMS.txt` elenca il checksum di ciascun file.

### Windows

| Build | Installer | Portable |
|---|---|---|
| x64 (Intel/AMD a 64 bit) | `craft-anything-<ver>-windows-x64.msi` | `craft-anything-<ver>-windows-x64-portable.zip` |
| arm64 (Snapdragon e altri PC ARM) | `craft-anything-<ver>-windows-arm64.msi` | `craft-anything-<ver>-windows-arm64-portable.zip` |

Gli installer e gli eseguibili sono code-signed. Lo zip portable conserva le impostazioni accanto all'app.

### macOS

| Build | File | Note |
|---|---|---|
| App universal (Apple Silicon + Intel) | `craft-anything-<ver>-macos-universal.dmg` | Signed e notarized |
| CLI tool universal | `craft-anything-cli-<ver>-macos-universal.zip` | Signed e notarized; vedi sotto se macOS lo blocca |

Le versioni successive alla 0.2.0 includono il tool da riga di comando nell'app, quindi per creare progetti
non serve lo zip separato. È necessario solo per usare la riga di comando senza l'app.

Lo zip scaricato dal browser può essere bloccato con il messaggio "Apple could not verify craft-anything-cli"
(un programma singolo non può portare con sé il ticket di notarization, quindi macOS deve verificarlo online).
Un download dal terminale non viene controllato in questo modo; puoi procedere così:

```sh
ver=0.2.0   # sostituisci con la versione desiderata
curl -fLO "https://github.com/storytold/craft-anything/releases/download/v$ver/craft-anything-cli-$ver-macos-universal.zip"
ditto -x -k "craft-anything-cli-$ver-macos-universal.zip" .
mkdir -p ~/.local/bin
mv "craft-anything-cli-$ver-macos-universal/craft-anything-cli" ~/.local/bin/
~/.local/bin/craft-anything-cli --version
```

Per eseguirlo semplicemente come `craft-anything-cli`, aggiungi `~/.local/bin` al tuo `PATH`.

Se hai già la copia bloccata, premi Fine (non Sposta nel Cestino), poi esegui
`xattr -d com.apple.quarantine <path to craft-anything-cli>`, oppure autorizzala in Impostazioni di Sistema →
Privacy e sicurezza → Apri comunque.

#### Verificare code signing e notarization di un download per Mac

Ogni release per Mac usa il code signing con un certificato Developer ID e la notarization di Apple. Per verificare un download,
esegui questi comandi nel Terminale (sostituisci `<ver>` con la versione e i percorsi con i tuoi):

```sh
# Disk image: "accepted" e "source=Notarized Developer ID" indicano che la verifica è riuscita
spctl --assess --type open --context context:primary-signature -v "craft-anything-<ver>-macos-universal.dmg"
# Il ticket di notarization è allegato: stampa "The validate action worked!"
xcrun stapler validate "craft-anything-<ver>-macos-universal.dmg"

# App, dopo aver aperto la disk image: la signature deve risultare valida
codesign --verify --deep --strict --verbose=2 "/Volumes/<disk image name>/Craft Anything.app"
spctl --assess --type execute --verbose "/Volumes/<disk image name>/Craft Anything.app"

# CLI tool: un programma singolo non può contenere un ticket, quindi macOS lo verifica online
codesign --verify --strict --verbose=2 ~/.local/bin/craft-anything-cli
codesign -dv --verbose=4 ~/.local/bin/craft-anything-cli 2>&1 | grep Authority
```

Le righe `Authority` devono indicare un certificato "Developer ID Application". Se una verifica fallisce,
non eseguire il file; segnala il problema includendo l'output e il tag della release.

### Linux

| Formato | x86_64 | aarch64 (ARM64) | Note |
|---|---|---|---|
| AppImage | `craft-anything-<ver>-linux-x86_64.AppImage` | `craft-anything-<ver>-linux-aarch64.AppImage` | Funziona ovunque; si aggiorna con [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) (file `.zsync`) |
| Flatpak | `craft-anything-<ver>-linux-x86_64.flatpak` | `craft-anything-<ver>-linux-aarch64.flatpak` | Isolato in una sandbox: non può avviare un browser sul computer, necessario per studiare i siti web (per questo usa un altro package); `flatpak install --user <file>` |
| Debian/Ubuntu | `craft-anything-<ver>-linux-x86_64.deb` | `craft-anything-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `craft-anything-<ver>-linux-x86_64.rpm` | `craft-anything-<ver>-linux-aarch64.rpm` | |
| Tarball | `craft-anything-<ver>-linux-x86_64.tar.gz` | `craft-anything-<ver>-linux-aarch64.tar.gz` | Estraibile ovunque |

Le build RISC-V (riscv64) sono disponibili come `craft-anything-<ver>-linux-riscv64.tar.gz`, `craft-anything-<ver>-linux-riscv64.deb` e `craft-anything-<ver>-linux-riscv64.rpm`.

L'AppImage contiene anche la CLI: esegui `<AppImage> --cli <command>` (per esempio
`<AppImage> --cli mcp` per un client MCP), oppure crea un collegamento all'AppImage chiamato
`craft-anything-cli` ed eseguilo.

### FreeBSD

| Build | File |
|---|---|
| x86_64 | `craft-anything-<ver>-freebsd-x86_64.tar.gz` |
| aarch64 (ARM64) | `craft-anything-<ver>-freebsd-aarch64.tar.gz` |

### Web (WebAssembly)

| Build | File | Note |
|---|---|---|
| Browser viewer | `craft-anything-web-<ver>.zip` | Identifica un file nel browser (il file non lo lascia mai); ospita la cartella su un qualsiasi server web statico ([packaging/web/README.md](packaging/web/README.md)) |

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Unisciti alla community ArtCraft su Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">Vieni a creare con noi</h3>

<p align="center">
  Su Discord si incontrano artisti di ogni tipo: persone che dipingono, scattano foto, disegnano, fanno video editing,
  si occupano di typography e persone che stanno ancora scoprendo cosa amano creare. Condividi ciò a cui stai lavorando,
  chiedi aiuto, segnala cosa non funziona oppure raccontaci cosa vorresti poter fare con questi tool.
  Qualunque sia il tuo mezzo espressivo e da quanto tempo lo usi, sei il benvenuto.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">Le Crafting Apps</a> ·
  <a href="https://github.com/storytold/craft-anything">Craft Anything</a>
</p>

---

## La suite di app Craft

| | App | A cosa serve | Codice | Per saperne di più |
|:-:|---|---|---|---|
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | Image editing: layer, mask, type e veri file PSD | [GitHub](https://github.com/storytold/photocraft) | [Sito web](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | Vector illustration | [GitHub](https://github.com/storytold/vectorcraft) | [Sito web](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | Video editing, colore e audio | [GitHub](https://github.com/storytold/filmcraft) | [Sito web](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | Photo library e RAW development | [GitHub](https://github.com/storytold/lightcraft) | [Sito web](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | Lettura, organizzazione e protezione dei PDF | [GitHub](https://github.com/storytold/pdfcraft) | [Sito web](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | Motion graphics ed effetti visivi | [GitHub](https://github.com/storytold/effectcraft) | [Sito web](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | Page layout e publishing | [GitHub](https://github.com/storytold/designcraft) | [Sito web](https://getartcraft.com/apps/designcraft) |
| <img src="https://raw.githubusercontent.com/storytold/figcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.figcraft.png" alt="" width="32" height="32"> | **FigCraft** | Interface design e prototyping | [GitHub](https://github.com/storytold/figcraft) | [Sito web](https://getartcraft.com/apps/figcraft) |
| <img src="https://raw.githubusercontent.com/storytold/wordcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.wordcraft.png" alt="" width="32" height="32"> | **WordCraft** | Elaborazione di testi e documenti | [GitHub](https://github.com/storytold/wordcraft) | [Sito web](https://getartcraft.com/apps/wordcraft) |
| <img src="https://raw.githubusercontent.com/storytold/soundcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.soundcraft.png" alt="" width="32" height="32"> | **SoundCraft** | Registrazione e audio editing | [GitHub](https://github.com/storytold/soundcraft) | [Sito web](https://getartcraft.com/apps/soundcraft) |
| <img src="https://raw.githubusercontent.com/storytold/deckcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.deckcraft.png" alt="" width="32" height="32"> | **DeckCraft** | Presentazioni e slide deck | [GitHub](https://github.com/storytold/deckcraft) | [Sito web](https://getartcraft.com/apps/deckcraft) |
| <img src="https://raw.githubusercontent.com/storytold/cadcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.cadcraft.png" alt="" width="32" height="32"> | **CADCraft** | Computer-aided design | [GitHub](https://github.com/storytold/cadcraft) | [Sito web](https://getartcraft.com/apps/cadcraft) |
| <img src="https://raw.githubusercontent.com/storytold/gridcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.gridcraft.png" alt="" width="32" height="32"> | **GridCraft** | Spreadsheet e dati | [GitHub](https://github.com/storytold/gridcraft) | [Sito web](https://getartcraft.com/apps/gridcraft) |
| <img src="https://raw.githubusercontent.com/storytold/texturecraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.texturecraft.png" alt="" width="32" height="32"> | **TextureCraft** | 3D texture painting | [GitHub](https://github.com/storytold/texturecraft) | [Sito web](https://getartcraft.com/apps/texturecraft) |
| <img src="https://raw.githubusercontent.com/storytold/materialcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.materialcraft.png" alt="" width="32" height="32"> | **MaterialCraft** | Node-based procedural materials | [GitHub](https://github.com/storytold/materialcraft) | [Sito web](https://getartcraft.com/apps/materialcraft) |
| <img src="https://raw.githubusercontent.com/storytold/craft-anything/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.craft-anything.png" alt="" width="32" height="32"> | **Craft Anything** | Scopri come funziona qualsiasi software, poi crea la tua versione | [GitHub](https://github.com/storytold/craft-anything) | [Sito web](https://getartcraft.com/apps/craft-anything) |
| <img src="https://raw.githubusercontent.com/tangency-labs/solidcraft/main/assets/icons/app/linux/exports/hicolor/64x64/apps/io.tangencylabs.solidcraft.png" alt="" width="32" height="32"> | **SolidCraft** | CAD 3D parametrico di Tangency Labs (app della community) | [GitHub](https://github.com/tangency-labs/solidcraft) | |

## Licenza e crediti

Craft Anything è distribuito con doppia licenza [MIT](LICENSE-MIT) oppure [Apache-2.0](LICENSE-APACHE), a tua scelta.
Copyright (c) 2026 ArtCraft Team e contributor di Craft Anything ([CONTRIBUTORS.md](CONTRIBUTORS.md)). Le note obbligatorie sono in [NOTICE](NOTICE).

Font, icone, immagini e altri asset inclusi mantengono le proprie licenze aperte; ogni asset è elencato
con autore, fonte e licenza in [ATTRIBUTION.md](ATTRIBUTION.md).

Gli screenshot mostrano progetti di esempio creati appositamente: example.com, un gioco Snake e una
piccola calcolatrice.

Il nome ArtCraft, il wordmark e i loghi in [`docs/brand/`](docs/brand/) sono marchi dell'ArtCraft Team e non sono
coperti da questa licenza. Possono essere usati solo senza modifiche e soltanto come parte di questo repository
e di Craft Anything, secondo [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
I fork e le versioni modificate devono rimuoverli.

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>Creato dal team e dalla community <a href="https://getartcraft.com/">ArtCraft</a>.</sub>
</p>
