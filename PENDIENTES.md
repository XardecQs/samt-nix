# Pendientes — GTA Mod Organizer (SAMT)

Estado a 2026-09-17. Solo lo que queda por hacer.

## Funcionalidad (del listado original)

- **Unificar variantes/configuraciones en una entrada**: modelo de "opciones por
  perfil" (un mod con subcarpetas seleccionables) para RoSA 4K/LITE, Proper
  Fixes, Loadscreens 2K/4K, Mobile Font y presets de Urbanize/Proper Shaders.
  Idea confirmada ("un mod con opciones"); falta implementarlo.
- **Visualizador gráfico del árbol de dependencias** estilo Obsidian, con
  `egui_graphs`.

## Empaquetado y manifiestos

- **Árbol homogéneo de extras** (`extras/{presets,variants,optional,docs,source}`):
  decidir taxonomía y nombres/idioma. Quedó "para discutir".
- **Variantes/variaciones** (`loadscreens-definitive/variaciones`,
  `mobile-font-fixed/variantes`, etc.): decidir si van a `options/<grupo>/<valor>/`
  de cara al feature futuro o a `extras/variants/`.
- **Renombrar `capturas/` → `screenshots/`** en los ~20 mods y actualizar
  `screenshots = ["screenshots"]`.
- **Migrar los ~39 mods legacy** (sin `mod.toml`, con contenido suelto o solo
  `modloader/`) a `content/` + `mod.toml`.
- **Auditoría de autoría/licencias** de los ~20 mods aún no revisados en su web
  (anotar términos de uso/permisos).
- **Decisión menor**: en `24h-timecycle` quedó `author = ["aap"]`, pero la web
  dice "The Hero" (misma persona); decidir si se deja `aap` o `The Hero`.
- (Propuesto, opcional) `gta-mo ctl lint`/`doctor` que valide ids, tags, rutas de
  `cover/guides/screenshots/mount` y `content/`.

## Discusiones abiertas

- Nombres/idioma del árbol de extras y su jerarquía con `content/`.

## A vigilar

- El `EPERM` al remontar tras abrir `run/merged` en el explorador: se asumió
  resuelto por el fix del guard (mount id + limpieza previa). Si reaparece,
  capturar `findmnt run/merged`, `/proc/self/mountinfo` y los procesos
  `umu-run`/`pressure-vessel`.

## Referencia rápida — lo ya hecho (para contexto)

- Overlay sin lanzar el juego: `gta-mo mount`/`unmount`/`status` (daemon con
  lock) + `--temp` (upper efímero fuera de la DB) y captura/descarte del upper;
  `ctl upper list/move/discard/capture`; pestaña **Overwrite** en la GUI.
- Bugs: orden de carga (Kahn por prioridad), abrir carpeta/URL durante el juego
  (out-of-band), guard de montaje por mount id, dedup de grupos.
- UI: diálogos centrados en la ventana y auto-dimensionados.
- Mods: ocultar (global y por perfil) + notas por perfil (schema v9).
- Manifiestos: campo `license` (schema v10); 27 `mod.toml` corregidos
  (ids, versiones, tags, dependencias, formato).
