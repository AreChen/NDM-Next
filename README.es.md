<div align="center">
  <img src="assets/readme/hero.webp" width="1200" alt="OnesDM, gestor de descargas con IA nativa. Un halcón peregrino en picado deja una estela de segmentos de descarga paralelos.">
  <h1><picture><source media="(prefers-color-scheme: dark)" srcset="assets/onesdm.svg"><img src="assets/onesdm-light.svg" width="36" alt=""></picture> OnesDM</h1>
  <p>Ones Download Manager · PlayOnes · AI-Native Download Manager</p>
  <p><strong>Any Agent. All Downloads.</strong></p>
  <p>Gestor de descargas nativo de IA para Windows, macOS y Linux. Está escrito en Rust con GPUI y combina ODM Core, un motor con seguridad de memoria, con complementos y clientes de IA opcionales.</p>
  <p><a href="https://github.com/AreChen/OnesDM/releases/latest"><strong>Descargar</strong></a> · <a href="https://odm.playones.com/es/">odm.playones.com</a></p>
</div>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-Hant.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <strong>Español</strong> · <a href="README.pt-BR.md">Português (Brasil)</a> · <a href="README.fr.md">Français</a> · <a href="README.de.md">Deutsch</a></p>

## Funciones

- Descargas HTTP(S) con varias conexiones y reanudación, además de FTP(S) y SFTP.
- Guarda contenido HLS y DASH estático. Busca audio y vídeo detectados en el navegador y descarga varios recursos a la vez.
- Busca por nombre, origen o URL, filtra por categoría y usa Mayús para seleccionar un intervalo. Abre los archivos desde la lista.
- Elige las carpetas de descargas y caché, libera espacio y decide qué proxy utilizar en cada tarea.
- Usa ventanas de progreso compactas y temas claros u oscuros. Un perfil nuevo elige un idioma compatible del sistema. Puedes cambiar entre nueve idiomas en **Ajustes → Apariencia e idioma**.

## Capturas de pantalla

### Tema claro · Configuración

[![El panel de configuración de OnesDM en el tema claro muestra la integración con IA, el idioma, los hilos de descarga y los ajustes de red.](assets/readme/downloads-light.webp)](assets/readme/downloads-light.png)

### Tema oscuro · Descargas

[![Cuatro descargas completadas en la lista de tareas de OnesDM con el tema oscuro.](assets/readme/downloads-dark.webp)](assets/readme/downloads-dark.png)

## Descargar

[Obtener el instalador en GitHub Releases](https://github.com/AreChen/OnesDM/releases/latest)

| Plataforma | Paquete | Instalación |
| --- | --- | --- |
| Windows · x64 | `OnesDM.exe` | Ejecuta el instalador. |
| macOS 12+ · Apple Silicon | `OnesDM-*-macos-arm64.dmg` | Arrastra OnesDM a Aplicaciones. |
| macOS 12+ · Intel | `OnesDM-*-macos-x64.dmg` | Arrastra OnesDM a Aplicaciones. |
| Linux · x86-64 | `OnesDM-*-linux-x64.tar.gz` | Instala las [dependencias de Linux](desktop/packaging/linux/README.md), extrae el archivo y ejecuta `ndm-desktop`. |

**Si usas 0.6.8, 0.6.9 o 0.7.0**, el actualizador omite los paquetes de la dirección actual de GitHub. Pausa o termina tus descargas, cierra la aplicación por completo e instala manualmente la versión 0.7.1 o posterior una vez. Conserva tu perfil para mantener el historial, los ajustes y los complementos.

Después podrás buscar actualizaciones desde la aplicación. OnesDM verifica el tamaño y el SHA-256 del paquete. Windows instala cuando terminan las tareas, macOS reemplaza la aplicación o abre el DMG si hace falta, y Linux abre la carpeta del archivo.

Un perfil nuevo permite hasta 16 conexiones HTTP por descarga. El tamaño de los segmentos depende del tamaño del archivo y del tiempo de respuesta inicial. Se respetan los ajustes guardados.

## Línea de comandos

Añade a PATH la carpeta que contiene `onesdm` o invoca el ejecutable por su ruta. En macOS está en `OnesDM.app/Contents/MacOS`. Puedes enviar una URL HTTP(S) a la cola del escritorio o iniciar el servidor MCP por entrada y salida estándar.

```text
onesdm --version
onesdm download <URL>
onesdm mcp
```

## Complementos e IA

<img src="assets/readme/plugins-ai.webp" width="1200" alt="Una tira de película, una onda de audio, una red entre pares y un terminal confluyen en una pila de archivos descargados.">

Instala **BitTorrent** desde el catálogo para descargar archivos .torrent y enlaces magnet. **Media Downloader** obtiene vídeo, audio y listas de reproducción de los sitios compatibles. [GitHub](https://github.com/AreChen/onesdm-plugin-market)

OpenAI Codex (CLI y escritorio), Claude Code, Pi, OpenCode y DeepSeek Harness pueden añadir y gestionar descargas con OnesDM MCP y su Skill. [Guía de complementos e IA](docs/plugins-and-ai.md)

## Extensión del navegador

Chrome, Edge y Brave pueden transferir descargas según el tipo, tamaño o dominio. Usa los botones junto al contenido o la biblioteca de recursos para seleccionar varios elementos. Cada versión incluye su ZIP de extensión. Elige OnesDM como destino. También admite Neat Download Manager. [Guía de instalación de la extensión](docs/extension-install.md)

[Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension)

<details>
<summary><strong>Compilación y publicación</strong></summary>

Al publicar una etiqueta desktop-vX.Y.Z, GitHub Actions compila Windows, las dos arquitecturas de macOS, Linux y la extensión. Los paquetes se publican al pasar las pruebas. [GitHub Actions](.github/workflows/release.yml)

[Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md)

</details>

---

[MIT](LICENSE) · [Privacidad](PRIVACY.md) · Proyecto independiente, sin afiliación con Neat Download Manager.
