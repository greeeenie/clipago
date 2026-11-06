# Clipago

<img src="app/assets/branding/clipago-app-icon.png" width="80" alt="Логотип Clipago">

Приложение для Windows, которое сохраняет последние секунды экрана и позволяет быстро обрезать клип или собрать несколько в одно видео.

## Установка

1. Скачай и запусти `Clipago_*_x64-setup.exe` из [последнего релиза](https://github.com/greeeenie/clipago/releases/latest).
2. Установи FFmpeg и FFprobe и добавь их в `PATH`.

**Ctrl + Shift + S** сохраняет недавний момент. Новые версии появляются в **Настройки → Обновления**.

## Разработка

Нужны Node.js, Rust и Visual Studio Build Tools с C++.

```powershell
cd app
npm ci
npm run tauri -- dev --no-default-features
```

Тег `vX.Y.Z` запускает сборку подписанного релиза в GitHub Actions. Версия должна совпадать в конфигах Cargo, npm и Tauri.

[Лицензия MIT](LICENSE)
