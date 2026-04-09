# Решения из обсуждения (2026-04-09)

## Стек
- **Native Rust**, без web-движков
- Tauri отвергнут: 150-200 MB RAM ради 3 инпутов (WebView2 = тот же Chromium)
- GUI-либа TBD: экспериментируем с egui, iced, или winapi
- Цель: ~10-20 MB RAM, мгновенный старт, один .exe

## UI — spotlight-стиль
- Borderless окно по центру экрана, поверх всех окон
- Похоже на Spotlight / Claude quick chat / Raycast
- 3 поля через Tab: "что делаю" → "не хуйню ли" → таймер (мин)
- Enter = сохранить + закрыть, Esc = закрыть без записи
- Фокус на первом поле при появлении

## Референсы
- Flow Launcher (C# WPF) и PowerToys Run — spotlight на Windows
- egui_overlay — Rust крейт для прозрачных overlay окон
- tauri-plugin-spotlight — архитектурный референс (не используем)

## Приоритет фич
1. MVP: popup + таймер + CSV журнал + трей
2. После MVP: горячая клавиша, звук, автозапуск
3. Далее: LLM-классификатор оценки, голосовой ввод (ElevenLabs STT)
