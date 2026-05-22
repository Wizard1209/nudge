# Nudge — interval journaling for focus

Периодический spotlight-popup, который спрашивает тебя: "что ты делаешь?" и "не хуйню ли ты делаешь?". Ответы пишутся в append-only NDJSON-журнал.

## Как работает

1. Nudge сидит в системном трее, тикает таймер
2. Таймер истёк → появляется spotlight-popup по центру экрана (поверх всего, как Spotlight / Claude quick chat)
3. Пользователь отвечает на вопросы → Enter → popup исчезает, таймер перезапускается
4. Ответы записываются в `journal-rust.ndjson`

## Popup

Минималистичное окно по центру экрана, поверх всех окон. Появляется с фокусом на первом поле.

### Поля (переключение через Tab)

| # | Поле | Тип | Описание |
|---|------|-----|----------|
| 1 | Что я делаю? | текст | Свободный ввод, фокус при открытии |
| 2 | Не хуйню ли я делаю? | текст | Свободный ввод, рефлексия |
| 3 | Следующий nudge через | число (мин) | Предзаполнено текущим интервалом (по умолчанию 10 мин) |

- **Enter** — сохранить и закрыть
- **Esc** — закрыть без сохранения (таймер всё равно перезапускается)

## Журнал

NDJSON файл (`journal-rust.ndjson`), append-only, одна JSON-запись на строку:

```jsonl
{"schema_version":1,"event_type":"submitted","entry_id":"01JS1S8R5W4Y4S4M8Q6A8X7R2V","captured_at":"2026-04-08T14:30:00.000+03:00","implementation":"rust","trigger_source":"timer","doing":"пишу требования к nudge","bullshit":"нет вроде норм","next_interval_minutes":10}
{"schema_version":1,"event_type":"submitted","entry_id":"01JS1S9FDRW4K4M7R4F5R9A5A2","captured_at":"2026-04-08T14:40:00.000+03:00","implementation":"rust","trigger_source":"timer","doing":"залип в ютуб","bullshit":"да","next_interval_minutes":5}
```

Полный контракт: [docs/journal-spec.md](docs/journal-spec.md)

## Трей

- Иконка в системном трее
- Тултип: `~N min` (округлено вверх, обновляется раз в минуту); `now` после истечения таймера
- Правый клик: пауза / настройки / выход
- TODO: показ оставшегося времени

## Стек

- **Rust** (native, без web-движков)
- GUI: TBD (egui / iced / winapi — выберем экспериментально)
- Цель: минимальный footprint (~10-20 MB RAM), мгновенный старт, один бинарник

## Дефолты

- Интервал: **10 минут** — настраивается в `config.json`
  (поле `default_interval_minutes`, любое положительное число).
- Журнал: `%USERPROFILE%\Documents\Nudge\journal-rust.ndjson`
- Хоткей: **Ctrl+Shift+Space** — вызывает popup вручную из любого окна.
  Настраивается в `%USERPROFILE%\Documents\Nudge\config.json` (поле `hotkey`).
  Формат: модификаторы (`Ctrl`, `Alt`, `Shift`, `Win`) + одна клавиша
  через `+`, например `Alt+J` или `Ctrl+F12`.

## TODO (после MVP)

- [ ] Звуковой сигнал при popup
- [ ] LLM-классификатор: автооценка "хуйня или нет" из текста обоих полей (если явно не указано → `null`). Лёгкая модель, локально или API
- [ ] Голосовой ввод через ElevenLabs STT — кнопка/хоткей в popup для диктовки вместо набора
