# StageSwap localization guide

StageSwap uses clear, natural product language in English, French, and neutral international Spanish. Translate the user’s task and the visible result, not the English sentence word for word. Keep labels concise, then use nearby explanatory copy when a concept needs more context.

## Approved terminology

| Concept | English | French | Spanish |
|---|---|---|---|
| Saved comparison screenshot | Reference image | Image de référence | Imagen de referencia |
| Monitored presentation display | Secondary screen | Écran secondaire | Pantalla secundaria |
| Final virtual-camera video | Zoom output | Sortie Zoom | Salida de Zoom |
| Main behavior | Automatic switching | Changement automatique | Cambio automático |
| Compact output modes | Auto / Camera / Screen | Auto / Caméra / Écran | Auto / Cámara / Pantalla |
| Physical camera input | Webcam | Webcam | Cámara web |
| No-content status | No media | Aucun média | Sin contenido multimedia |
| Content status | Media detected | Média détecté | Contenido multimedia detectado |
| Detection heading | Media detection | Détection des médias | Detección de contenido multimedia |
| Reference action | Capture reference image | Capturer l’image de référence | Capturar imagen de referencia |
| Comparison control | Required similarity | Similarité requise | Similitud requerida |
| Mode selector | Output mode | Mode de sortie | Modo de salida |
| Rescan and restart section | Tools | Outils | Herramientas |
| Camera selected in Zoom | Virtual camera | Caméra virtuelle | Cámara virtual |
| Windows background area | System tray | Zone de notification | Área de notificación |
| Onboarding flow | Guided setup | Configuration guidée | Configuración guiada |

## Context rules

- Use **Secondary screen** in page titles and explanations. Use **Screen** only in compact mode controls where it appears beside **Auto** and **Camera**.
- Use **Webcam** for the physical device. Use **Camera** only for the compact forced-output mode.
- Describe the reference condition directly: “JW Library is not playing media,” “aucun média n’est affiché dans JW Library,” and “JW Library no reproduce contenido multimedia.”
- In short French states, prefer **Aucun média** and **Média détecté**. In standalone explanatory labels, **Aucun média affiché** is also approved. Do not force *contenu* into French when *média* is sufficient.
- Adapt verbs to the language instead of concatenating glossary entries. The automatic-switching actions are **Start/Stop automatic switching**, **Activer/Désactiver le changement automatique**, and **Activar/Desactivar el cambio automático**.
- Keep **StageSwap**, **JW Library**, and **Zoom** unchanged.

## Voice and mechanics

- Use sentence case except for deliberate all-caps preview labels.
- Address French users with the formal **vous** register.
- Use direct, informal singular instructions in neutral international Spanish.
- Prefer familiar outcomes over implementation terms. User-facing copy should not describe the video pipeline, retransmission, recovery machinery, or an idle-reference implementation detail when a clearer task-oriented phrase is available.
- Preserve placeholders such as `{0}` and `{1}`, meaningful line breaks, and ellipses that indicate a following dialog.
- When one English phrase appears in different contexts, give each context its own source key instead of forcing one translation to serve incompatible grammatical roles.
