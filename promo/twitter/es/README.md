# Conclave MD — imágenes promo para X/Twitter (español)

Set visual en español con mockups de dispositivo, captura del producto y conceptos
de marketing. Rediseñado e iterado en dos rondas con crítica experta (estrategia de
marketing + dirección de arte + mockups/legibilidad). Renderizado desde HTML/CSS con
los tokens de diseño reales de la app (paleta monocromática Bugatti, wordmark
JetBrains Mono, mark "C" con nodos cyan, MacBook realista con bisel/cámara/sombra de
contacto). In-stream **16:9 @2x (3200×1800)**, cuadrado **2160²**, banner **3000×1000**.

## Estrategia de lanzamiento (hilo recomendado)

El principio rector: **liderar con tensión/contraste, no con descripción**. Cada pieza
desarma una de las cuatro objeciones del médico escéptico (¿es un chat más? / ¿se lo
inventa? / ¿y los datos del paciente? / ¿qué motor uso?).

| # | Pieza | Rol en el hilo |
|---|-------|----------------|
| **1 · FIJADO** | `es-08-contraste` | El gancho probatorio: chat (afirma, sin fuente) **vs** comité (delibera, cita, banderas rojas). *Demuestra* el posicionamiento en vez de afirmarlo. |
| 2 | `es-01-hero` | El "qué es": *Una segunda opinión que discute la primera* + producto real. |
| 3 | `es-03-deliberacion` | El "cómo": las 4 fases con el loop *vuelve y reta*. |
| 4 | `es-09-cita` | Mata la objeción nº1 (alucinación): cita documento + página exactos. |
| 5 | `es-04-privacidad` | Objeción nº2: los datos del paciente no salen del equipo. |
| 6 | `es-05-motores` | Objeción del dev: trae tu motor, o 100 % offline. |
| 7 | `es-02-captura` | Cierre con el producto en limpio. **Aquí añade el disclaimer** del hilo. |

`es-06` (multidispositivo) y `es-07` (1:1) son para cross-post (LinkedIn / Instagram) o
como alternativa al hero. `es-header` es el banner de perfil.

> **Disclaimer del hilo (tweet de cierre):** *Conclave MD es apoyo a la decisión clínica,
> no un dispositivo médico, y no sustituye el juicio del profesional.* (También aparece
> en micro en `es-09`.)

## Las piezas

| Archivo | Tamaño | Titular | Concepto |
|---------|--------|---------|----------|
| `es-08-contraste.png` | 3200×1800 | *Una respuesta no es una decisión.* | Chat vs comité, lado a lado. **Fijado.** |
| `es-01-hero.png` | 3200×1800 | *Una segunda opinión que discute la primera.* | Hero con MacBook. |
| `es-03-deliberacion.png` | 3200×1800 | *No responde. Lo pone a prueba.* | Las 4 fases + loop *vuelve y reta*. |
| `es-09-cita.png` | 3200×1800 | *Cada recomendación cita el documento y la página.* | Anti-alucinación. |
| `es-04-privacidad.png` | 3200×1800 | *Los datos del paciente no salen de tu equipo.* | Privacidad (candado cerrado). |
| `es-05-motores.png` | 3200×1800 | *Tu motor. Tus claves. O 100 % offline.* | Proveedores (fila offline destacada). |
| `es-02-captura.png` | 3200×1800 | *Mira cómo se construye el veredicto.* | Captura del producto en vivo. |
| `es-06-multidispositivo.png` | 3200×1800 | *El mismo comité. En cualquier equipo.* | macOS · Windows · Linux. |
| `es-07-cuadrado.png` | 2160×2160 | *No responde. Lo pone a prueba.* | 1:1 para IG / LinkedIn. |
| `es-header.png` | 3000×1000 | — | Banner de perfil de X. |

## Regenerar

Fuentes en `../src/` (comparten `style.css`). La captura del producto vive en
`src/_screen.html` y se inyecta en el marcador `__SCREEN__`; el MacBook realista es un
componente compartido en `style.css`.

```sh
cd promo/twitter/src
python3 build.py                       # todo
python3 build.py es-08-contraste.html  # una página
```

Render con Google Chrome headless a `--force-device-scale-factor=2`. Las fuentes
(JetBrains Mono / Inter) se cargan de Google Fonts al renderizar (el build necesita red).
