# TODO

Актуальный бэклог. Post-MVP фичи (хоткей, звук, автозапуск, LLM, голос) — в `README.md`.

## Рассинхрон кода со спекой

- [ ] **Frosted-glass backdrop на Windows.**
  Спека §8 сама помечает как открытое: acrylic/mica за карточкой через eframe. Сейчас только `Color32::TRANSPARENT` для panel/window — настоящего blur за окном нет.

## Технический долг

- [ ] **Optional поля в журнале.**
  `src/journal.rs:16` TODO: `prompt_version`, `input_method`, `metadata`. Сейчас не сериализуются. Контракт в `docs/journal-spec.md`.
