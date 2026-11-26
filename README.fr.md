<div align="center">
  <img src="assets/readme/hero.webp" width="1200" alt="OnesDM, gestionnaire de téléchargements à IA native. Un faucon pèlerin en piqué laisse derrière lui des segments de téléchargement parallèles.">
  <h1><picture><source media="(prefers-color-scheme: dark)" srcset="assets/onesdm.svg"><img src="assets/onesdm-light.svg" width="36" alt=""></picture> OnesDM</h1>
  <p>Ones Download Manager · PlayOnes · AI-Native Download Manager</p>
  <p><strong>Any Agent. All Downloads.</strong></p>
  <p>Gestionnaire de téléchargements conçu pour l’IA, sous Windows, macOS et Linux. Écrit en Rust avec GPUI, il associe la sûreté mémoire d’ODM Core à des moteurs extensibles et à une intégration IA facultative.</p>
  <p><a href="https://github.com/AreChen/OnesDM/releases/latest"><strong>Télécharger</strong></a> · <a href="https://odm.playones.com/fr/">odm.playones.com</a></p>
</div>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-Hant.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.pt-BR.md">Português (Brasil)</a> · <strong>Français</strong> · <a href="README.de.md">Deutsch</a></p>

## Fonctionnalités

- Téléchargements HTTP(S) à plusieurs connexions avec reprise, ainsi que FTP(S) et SFTP.
- Enregistrez des flux HLS et du DASH statique. Recherchez les vidéos et pistes audio détectées par le navigateur et téléchargez-les par lots.
- Recherchez un nom, une source ou une URL, filtrez par catégorie et sélectionnez une plage avec Maj. Ouvrez les fichiers depuis la liste.
- Choisissez les dossiers de téléchargement et de cache, libérez de l’espace et réglez le proxy par tâche.
- Fenêtres de progression compactes, modes clair et sombre et neuf langues. Un nouveau profil reprend une langue système compatible. Modifiez-la dans **Paramètres → Apparence et langue**.

## Captures d’écran

### Thème clair

[![Quatre téléchargements terminés dans la liste des tâches d’OnesDM en mode clair.](assets/readme/downloads-light.webp)](assets/readme/downloads-light.png)

### Thème sombre

[![Quatre téléchargements terminés dans la liste des tâches d’OnesDM en mode sombre.](assets/readme/downloads-dark.webp)](assets/readme/downloads-dark.png)

## Télécharger

[Obtenir le paquet sur GitHub Releases](https://github.com/AreChen/OnesDM/releases/latest)

| Plateforme | Paquet | Installation |
| --- | --- | --- |
| Windows · x64 | `OnesDM.exe` | Lancez l’installeur. |
| macOS 12+ · Apple Silicon | `OnesDM-*-macos-arm64.dmg` | Glissez OnesDM dans Applications. |
| macOS 12+ · Intel | `OnesDM-*-macos-x64.dmg` | Glissez OnesDM dans Applications. |
| Linux · x86-64 | `OnesDM-*-linux-x64.tar.gz` | Installez les [dépendances Linux](desktop/packaging/linux/README.md), extrayez l’archive et lancez `ndm-desktop`. |

**Depuis les versions 0.6.8, 0.6.9 ou 0.7.0**　Ces versions ignorent les paquets à l’adresse GitHub actuelle. Mettez les téléchargements en pause ou attendez leur fin, quittez complètement l’application et installez une fois manuellement la version 0.7.1 ou ultérieure. Gardez votre profil pour conserver historique, paramètres et extensions.

Vous pouvez ensuite rechercher les mises à jour dans l’application. OnesDM vérifie la taille du paquet et son SHA-256. Windows installe la mise à jour à la fin des tâches, macOS remplace l’application ou ouvre le DMG si nécessaire, et Linux ouvre le dossier de l’archive.

Un nouveau profil autorise jusqu’à 16 connexions HTTP par téléchargement. La taille des segments dépend de la taille du fichier et du temps de réponse initial. Les réglages existants sont conservés.

## Ligne de commande

Ajoutez le dossier contenant `onesdm` au PATH ou utilisez son chemin complet. Sous macOS, il se trouve dans `OnesDM.app/Contents/MacOS`. Envoyez une URL HTTP(S) à la file de l’application ou démarrez MCP sur l’entrée et la sortie standard.

```text
onesdm --version
onesdm download <URL>
onesdm mcp
```

## Extensions et IA

<img src="assets/readme/plugins-ai.webp" width="1200" alt="Une pellicule, une onde sonore, un réseau de pairs et une invite de terminal convergent vers une pile de fichiers téléchargés.">

Installez **BitTorrent** depuis le catalogue pour les fichiers .torrent et les liens magnet. **Media Downloader** récupère les vidéos, pistes audio et listes de lecture des sites compatibles. [GitHub](https://github.com/AreChen/onesdm-plugin-market)

Gérez les téléchargements depuis OpenAI Codex (CLI et bureau), Claude Code, Pi, OpenCode ou DeepSeek Harness grâce à OnesDM MCP et au Skill associé. [Guide des extensions et de l’IA](docs/plugins-and-ai.md)

## Extension de navigateur

Chrome, Edge et Brave peuvent transmettre les téléchargements selon le type, la taille et le domaine. Les boutons près des médias et la bibliothèque de ressources permettent les téléchargements par lots. Chaque version fournit son ZIP d’extension. Sélectionnez OnesDM comme gestionnaire. Neat Download Manager est également pris en charge. [Guide d’installation de l’extension](docs/extension-install.md)

[Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension)

<details>
<summary><strong>Compilation et publication</strong></summary>

Une étiquette desktop-vX.Y.Z déclenche sur GitHub Actions la compilation de Windows, des deux architectures macOS, de Linux et de l’extension. Les paquets sont publiés après la réussite des tests. [GitHub Actions](.github/workflows/release.yml)

[Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md)

</details>

---

[MIT](LICENSE) · [Confidentialité](PRIVACY.md) · Projet indépendant, sans affiliation avec Neat Download Manager.
