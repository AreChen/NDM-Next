<div align="center">
  <img src="assets/readme/hero.webp" width="1200" alt="OnesDM, gerenciador de downloads com IA nativa. Um falcão-peregrino em mergulho deixa um rastro de segmentos de download paralelos.">
  <h1><picture><source media="(prefers-color-scheme: dark)" srcset="assets/onesdm.svg"><img src="assets/onesdm-light.svg" width="36" alt=""></picture> OnesDM</h1>
  <p>Ones Download Manager · PlayOnes · AI-Native Download Manager · <a href="https://odm.playones.com/pt-br/">odm.playones.com</a></p>
  <p><strong>Any Agent. All Downloads.</strong></p>
  <p>Gerenciador de downloads nativo de IA para Windows, macOS e Linux. Feito em Rust e GPUI, combina a segurança de memória do ODM Core com plugins de download e clientes de IA opcionais.</p>
  <p><a href="https://github.com/AreChen/OnesDM/releases/latest"><strong>Baixar</strong></a></p>
</div>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-Hant.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <strong>Português (Brasil)</strong> · <a href="README.fr.md">Français</a> · <a href="README.de.md">Deutsch</a></p>

## Recursos

- Downloads HTTP(S) com múltiplas conexões e retomada, além de FTP(S) e SFTP.
- Baixe HLS e DASH estático. Encontre áudio e vídeo detectados pelo navegador e baixe vários recursos de uma só vez.
- Pesquise por nome, origem ou URL, filtre por tipo e use Shift para selecionar um intervalo. Abra arquivos diretamente na lista.
- Defina as pastas de downloads e cache, libere espaço e escolha o proxy de cada tarefa.
- Janelas compactas de progresso, temas claros e escuros e nove idiomas. Um perfil novo usa um idioma compatível do sistema. Para mudar, abra **Configurações → Aparência e idioma**.

## Capturas de tela

| Tema claro · Configurações | Tema escuro · Downloads |
| :---: | :---: |
| [![O painel de configurações do OnesDM no tema claro mostra a integração com IA, o idioma, as conexões de download e os ajustes de rede.](assets/readme/downloads-light.webp)](assets/readme/downloads-light.png) | [![Quatro downloads concluídos na lista de tarefas do OnesDM com o tema escuro.](assets/readme/downloads-dark.webp)](assets/readme/downloads-dark.png) |

## Baixar

[Baixar o instalador no GitHub Releases](https://github.com/AreChen/OnesDM/releases/latest)

| Plataforma | Pacote | Instalação |
| --- | --- | --- |
| Windows · x64 | `OnesDM.exe` | Execute o instalador. |
| macOS 12+ · Apple Silicon | `OnesDM-*-macos-arm64.dmg` | Arraste o OnesDM para Aplicativos. |
| macOS 12+ · Intel | `OnesDM-*-macos-x64.dmg` | Arraste o OnesDM para Aplicativos. |
| Linux · x86-64 | `OnesDM-*-linux-x64.tar.gz` | Instale as [dependências do Linux](desktop/packaging/linux/README.md), extraia o arquivo e execute `ndm-desktop`. |

**Atualização das versões 0.6.8, 0.6.9 e 0.7.0**　Essas versões ignoram os pacotes no endereço atual do GitHub. Pause ou conclua os downloads, encerre o aplicativo por completo e instale manualmente a versão 0.7.1 ou posterior uma vez. Mantenha o perfil para preservar histórico, configurações e plugins.

Depois disso, verifique atualizações no aplicativo. O OnesDM confere o tamanho e o SHA-256 do pacote. O Windows instala após o término das tarefas. O macOS substitui o aplicativo ou abre o DMG quando necessário. O Linux abre a pasta do arquivo.

Perfis novos permitem até 16 conexões HTTP por download. O tamanho dos segmentos leva em conta o tamanho do arquivo e o tempo da primeira resposta. As configurações existentes são preservadas.

## Linha de comando

Adicione ao PATH a pasta que contém o comando `onesdm` ou execute-o pelo caminho completo. No macOS, ele fica em `OnesDM.app/Contents/MacOS`. É possível enviar uma URL HTTP(S) à fila do aplicativo ou iniciar o MCP pela entrada e saída padrão.

```text
onesdm --version
onesdm download <URL>
onesdm mcp
```

## Plugins e IA

<img src="assets/readme/plugins-ai.webp" width="1200" alt="Uma tira de filme, uma onda de áudio, uma rede de pares e um terminal convergem para uma pilha de arquivos baixados.">

Instale **BitTorrent** no catálogo para baixar arquivos .torrent e links magnet. O **Media Downloader** baixa vídeos, áudios e playlists de sites compatíveis. [GitHub](https://github.com/AreChen/onesdm-plugin-market)

Adicione e gerencie downloads pelo OpenAI Codex (CLI e desktop), Claude Code, Pi, OpenCode ou DeepSeek Harness com o OnesDM MCP e seu Skill. [Guia de plugins e IA](docs/plugins-and-ai.md)

## Extensão do navegador

Chrome, Edge e Brave podem encaminhar downloads por tipo, tamanho e domínio. Use os botões junto à mídia ou selecione recursos em lote na biblioteca. Cada versão inclui seu ZIP de extensão. Selecione OnesDM como destino. A extensão também aceita Neat Download Manager. [Guia de instalação da extensão](docs/extension-install.md)

[Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension)

<details>
<summary><strong>Compilação e publicação</strong></summary>

Uma tag desktop-vX.Y.Z aciona o GitHub Actions para compilar Windows, as duas arquiteturas do macOS, Linux e a extensão. Os pacotes são publicados quando os testes passam. [GitHub Actions](.github/workflows/release.yml)

[Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md)

</details>

---

[MIT](LICENSE) · [Privacidade](PRIVACY.md) · Projeto independente, sem vínculo com o Neat Download Manager.
