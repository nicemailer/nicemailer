# NiceMailer : thèmes

Statut : à valider (22 novembre 2026). Complète `ui.md` (« Un thème est une présentation »), `configuration.md` (un dossier de fichiers de configuration de plus) et `raccourcis.md`, dont il reprend les règles de lecture.

## 1. Objet

Ce document spécifie comment un thème de l'app Swift est entièrement décrit par un fichier : le dossier `themes/`, le format des fichiers, l'héritage entre thèmes, le catalogue de ce qu'un thème peut régler, et la façon dont le noyau les lit et dont l'app s'en sert. Les trois thèmes livrés, geek, clean et regular, sont décrits de cette façon ; un utilisateur peut en modifier un ou créer le sien.

## 2. Principes

- **Un thème est une feuille de style.** Il règle la présentation : couleurs, polices, proportions, arrondis, animations, et choisit parmi des variantes que l'app sait dessiner (barre de statut ou barre d'outils, blocs montrés, avatars…). Il ne décrit pas une disposition libre : chaque variante est du code de l'app, écrit et testé. Le catalogue (section 6) s'enrichit avec l'app ; un fichier ne peut rien demander qui n'y figure pas.
- **Un thème ne retire aucune fonction.** Il peut masquer un bloc de la barre ou de la barre d'outils, jamais une fonction : la touche du bloc masqué marche toujours, son menu s'ouvre comme d'habitude. Désactiver une fonction reste un réglage de `config.toml` (`ai.enabled`, par exemple).
- **Un thème ne contient que des données.** Pas de code, pas d'expression, pas d'image ni de police à charger, rien qui désigne une adresse distante ou un fichier. Un thème reçu de quelqu'un peut donc être posé dans le dossier sans risque. Embarquer une police ou une image dans un thème pourra venir plus tard ; ce n'est pas prévu dans cette version.
- **La taille du texte n'appartient pas au thème.** Elle dépend des yeux de la personne : elle reste dans `config.toml` (`ui.text_size`, `ui.status_bar_text_size`) et ⌘+ ⌘− ⌘0 la changent comme aujourd'hui. Le thème ne donne que des proportions (section 5.3).
- **Mêmes touches pour tous les thèmes.** Le clavier ne dépend pas du thème (`raccourcis.md`) ; un thème choisit seulement si les blocs rappellent leur touche.
- **Le noyau lit les fichiers.** Comme `config.toml` et `keymap.toml`, c'est le noyau qui lit, valide, résout l'héritage et surveille les fichiers ; l'app reçoit le thème résolu par le contrat (section 8). L'app ne fait que deux choses que le noyau ne peut pas faire : traduire une couleur du système et vérifier qu'une police est installée sur le Mac (section 7).
- **Pour l'utilisateur avancé, par des fichiers.** Les Préférences choisissent un thème parmi ceux du dossier (section 9) ; elles ne les modifient pas. On modifie un thème en éditant son fichier, que le noyau crée commenté.

## 3. Emplacement, création, lecture

- Les thèmes sont dans **`themes/`, dans le dossier de `config.toml`** (`~/.config/nicemailer/themes/`, ou à côté du fichier désigné par `NICEMAILER_CONFIG`). Un thème est un fichier `<nom>.toml` ; son nom est celui du fichier, en minuscules, chiffres et tirets (`mon-theme`). Un fichier dont le nom ne suit pas cette règle est ignoré avec un avertissement.
- **Thèmes livrés.** `geek`, `clean` et `regular` sont embarqués dans le noyau : ils existent même sans fichier.
- **Création au démarrage.** Le dossier et les trois fichiers `geek.toml`, `clean.toml`, `regular.toml` sont créés s'ils manquent, selon les règles de `configuration.md`, section 3 : les en-têtes de tables ne sont pas commentés, chaque clé est écrite commentée avec sa valeur et son rôle (`# unread = "system.accent"   # point des mails non lus`). Il suffit d'enlever le `# `. Un fichier existant n'est jamais modifié. Le fichier créé ne change donc rien : il montre ce qu'on peut changer, et une valeur que l'utilisateur a décommentée reste la sienne quand une version suivante change la valeur livrée.
- **Lecture** : au démarrage, puis à chaque changement du dossier (FSEvents), sans redémarrage. Les événements d'un enregistrement sont groupés comme pour `config.toml`.
- **Indépendant des autres fichiers.** Un thème invalide ne fait refuser ni `config.toml`, ni `accounts.toml`, ni `keymap.toml`, ni les autres thèmes. Une faute de couleur ne doit pas désactiver les comptes.
- **Fichier invalide : refusé en entier.** Le noyau garde la version précédente de ce thème. Au démarrage, sans version précédente : un thème livré modifié par le fichier reste le thème livré ; un thème de l'utilisateur, dont on ne sait plus ce qu'il étend, laisse la place à geek, avec un avertissement et émet `ConfigError {file: "themes/clean.toml", line, key, message}`. L'app l'affiche en rouge dans la barre de statut (ou le sous-titre de la barre d'outils), avec la ligne, jusqu'à ce que le fichier soit corrigé. Sont invalides :
  - une syntaxe TOML incorrecte ;
  - une table inconnue ;
  - une valeur du mauvais type, hors de ses bornes ou absente de ses choix (section 6) ;
  - une couleur mal écrite ou un nom de couleur du système inconnu (section 5.1) ;
  - un `extends` vers un thème qui n'existe pas, ou un héritage circulaire, ou plus de 8 niveaux ;
  - dans `blocks`, un bloc inconnu ou cité deux fois.
- **Clé inconnue : avertissement**, pas erreur. La ligne est ignorée et `nicemailer theme check` la signale. Un thème écrit pour une version plus récente de NiceMailer reste lisible par une plus ancienne.
- **Version du format** : `version = 1`.
- **Taille** : un fichier de plus de 64 Ko est refusé (un thème en fait quelques-uns ; la borne protège le noyau d'un fichier qui n'en est pas un).

## 4. Héritage

- Chaque thème a un parent, dont il reprend toutes les valeurs ; le fichier ne contient que les différences.
- **Un fichier qui porte le nom d'un thème livré le modifie** : `regular.toml` a pour parent le thème `regular` embarqué. Il ne peut pas déclarer d'autre parent.
- **Tout autre fichier déclare son parent** : `extends = "clean"`, un thème livré ou un autre thème du dossier. Sans `extends`, le parent est `geek`, avec un avertissement.
- Le parent est résolu tel qu'il est, modifications de l'utilisateur comprises : un thème qui étend `clean` suit ce que l'utilisateur a changé dans `clean.toml`.
- Un thème dont le parent est refusé se résout sur la dernière version valide de ce parent.
- L'héritage se fait clé par clé. Une table (`[colors]`) n'est pas remplacée en bloc : seules les clés écrites changent. Seule exception, `blocks` (section 6.1), une liste, qui se remplace en entier.

## 5. Valeurs

### 5.1 Couleurs

Une couleur s'écrit de trois façons :

- **`"#RRGGBB"`** ou **`"#RRGGBBAA"`** : la même couleur dans les deux apparences ;
- **`{ light = "#FAFAF8", dark = "#1B1D23" }`** : une couleur par apparence de macOS ; les deux sont obligatoires ;
- **`"system.<nom>"`** : une couleur du système, que macOS adapte lui-même à l'apparence, au contraste augmenté et à la couleur d'accent choisie dans Réglages Système. Noms acceptés : `accent`, `label`, `secondary_label`, `tertiary_label`, `separator`, `window_background`, `control_background`, `under_page_background`, `text_background`, `selected_content_background`, `unemphasized_selected_content_background`, `alternating_content_background`, `find_highlight`, `red`, `orange`, `yellow`, `green`, `blue`, `purple`, `gray`.

Le noyau vérifie l'écriture ; l'app traduit les noms du système (section 7).

### 5.2 Polices

Une police s'écrit :

- **`"system"`** (SF Pro), **`"system-mono"`** (SF Mono), **`"system-serif"`** (New York), **`"system-rounded"`** (SF Pro Rounded) ;
- ou **le nom d'une famille** : `"iA Writer Mono V"` (livrée avec l'app), `"Iosevka"`, `"Charter"`. La famille doit être installée sur le Mac ou livrée avec l'app ; sinon l'app prend `"system"` (ou `"system-mono"` si la clé exige une chasse fixe, ce qu'aucune n'exige aujourd'hui) et le signale (section 7).

### 5.3 Proportions

La taille du texte vient de `config.toml`. Le thème donne :

- des **facteurs** de taille de texte (`reading_scale = 1.08` : le corps des mails 8 % plus grand que l'interface) ;
- des **longueurs en points, écrites pour un texte de 12 pt** (hauteur de ligne, arrondis, marges), que l'app multiplie par `ui.text_size / 12`, exactement comme les longueurs de geek aujourd'hui (`TextSize.referenceSize`). Un thème garde ainsi ses proportions à toutes les tailles.
- Les **durées** sont en millisecondes et ne dépendent pas de la taille. L'app les met à 0 quand Réglages Système demande de réduire les animations.

## 6. Catalogue

Chaque clé a son type, ses bornes ou ses choix, et sa valeur dans les trois thèmes livrés. Les valeurs ci-dessous sont celles des maquettes validées en novembre 2026 ; les fichiers embarqués font foi, et un test vérifie qu'ils reprennent cette section.

### 6.1 `[window]` : la fenêtre et sa commande

| Clé | Choix | geek | clean | regular | Rôle |
| --- | --- | --- | --- | --- | --- |
| `chrome` | `status_bar`, `toolbar` | `status_bar` | `status_bar` | `toolbar` | Commande de la fenêtre : barre de statut compacte, ou barre d'outils macOS dont chaque bouton est un bloc |
| `status_bar_position` | `top`, `bottom`, `hidden` | `top` | `top` | — | Position par défaut de la barre de statut ; `v` la change ensuite pour la fenêtre (`ui.md`, « Préférences d'affichage ») |
| `title_bar` | `shown`, `hidden`, `merged` | `shown` | `merged` | `merged` | `hidden` : ni barre de titre ni boutons (le contenu monte jusqu'en haut, comme `ui.title_bar = "hidden"` aujourd'hui). `merged` : pas de barre de titre, les boutons rouge, jaune, vert sont dans la barre de statut ou la barre d'outils ; barre de statut en bas ou masquée : `merged` vaut `shown` |
| `key_hints` | booléen | `true` | `false` | `false` | Les blocs rappellent leur touche (`@`, `;`…) ; les touches marchent dans tous les cas |
| `block_labels` | booléen | `false` | `true` | `true` | Les blocs d'action disent leur nom (Affichage, Déplacer, Enregistrer, IA…) à côté de leur icône |
| `status_bar_height` | 14 à 48 | 20 | 30 | 30 | Hauteur de la barre de statut, en points pour un texte de barre de 12 pt |
| `blocks` | liste | voir dessous | | | Blocs montrés, dans l'ordre |
| `default_columns` | 1, 2, 3 | 2 | 2 | 3 | Nombre de colonnes d'une première fenêtre |

`blocks` liste les blocs de la barre de statut ou de la barre d'outils : `mode`, `account`, `folder`, `attributes`, `view`, `move`, `save`, `ai`, `help`, et deux séparateurs, `"|"` (un large espace) et `"->"` (tout l'espace restant : ce qui suit va à droite). Un bloc absent est masqué, pas désactivé (section 2). Valeur des trois thèmes livrés : `["mode", "account", "folder", "attributes", "|", "view", "|", "move", "save", "ai", "->", "help"]` ; regular n'a pas `mode`. Trois éléments ne sont pas des blocs et ne peuvent pas être masqués, car ils disent ce que l'app fait : les messages (résultat d'une action, erreur, fichier de configuration refusé), l'avancement de la synchronisation et la question en attente. Avec la barre de statut, ils restent à son extrémité droite ; avec la barre d'outils, ils vont dans le sous-titre, sous le nom du dossier, comme l'avancement dans Mail.

### 6.2 `[fonts]`

| Clé | Type | geek | clean | regular | Rôle |
| --- | --- | --- | --- | --- | --- |
| `interface` | police | `"iA Writer Mono V"` | `"system"` | `"system"` | Toute l'interface : liste, barres, menus, sélecteurs |
| `reading` | police | `"iA Writer Mono V"` | `"system"` | `"system"` | Corps des mails en texte, et police par défaut du HTML assaini qui n'en donne pas |
| `reading_scale` | 0,75 à 1,5 | 1,0 | 1,15 | 1,08 | Taille du corps des mails par rapport à `ui.text_size` |
| `reading_line_spacing` | 0 à 16 | 3 | 6 | 4 | Points ajoutés entre deux lignes du corps des mails, quelle que soit la taille du texte (geek : les 3 points de son lecteur) |
| `title_scale` | 0,75 à 2,0 | 1,0 | 1,7 | 1,3 | Taille du sujet en tête du lecteur |

Le texte de la barre de statut garde sa taille propre (`ui.status_bar_text_size`).

### 6.3 `[layout]`

Longueurs en points pour un texte de 12 pt (section 5.3).

| Clé | Type | geek | clean | regular | Rôle |
| --- | --- | --- | --- | --- | --- |
| `row_height` | 14 à 48 | 20 | 30 | 24 | Hauteur d'une ligne de la liste |
| `row_radius` | 0 à 16 | 0 | 8 | 6 | Arrondi de la ligne choisie |
| `row_inset` | 0 à 16 | 0 | 8 | 8 | Retrait de la ligne choisie par rapport aux bords de la liste |
| `zebra` | booléen | `false` | `false` | `true` | Lignes alternées |
| `menu_row_height` | 0, ou 14 à 40 | 0 | 26 | 22 | Hauteur d'une ligne des menus et sélecteurs ; 0 : la hauteur de son texte (geek) |
| `popup_radius` | 0 à 24 | 0 | 14 | 9 | Arrondi des menus, sélecteurs, questions et panneaux |
| `popup_shadow` | booléen | `false` | `true` | `true` | Ombre portée sous ces panneaux |
| `menu_keys` | `left`, `right` | `left` | `right` | `right` | Place de la touche sur une ligne de menu |
| `message_cards` | booléen | `false` | `false` | `true` | Chaque message de la conversation dans un cadre arrondi |
| `avatars` | booléen | `false` | `false` | `true` | Initiales de l'expéditeur devant chaque message (rien n'est chargé) |
| `reader_max_width` | 0, ou 320 à 1200 | 0 | 560 | 0 | Largeur maximale du texte du lecteur, centré ; 0 : toute la largeur |
| `scrollers` | `none`, `thin`, `standard` | `none` | `thin` | `standard` | Ascenseurs |
| `separators` | `lines`, `space` | `lines` | `space` | `lines` | Zones séparées par des traits, ou surtout par du blanc |

### 6.4 `[motion]`

| Clé | Type | geek | clean | regular | Rôle |
| --- | --- | --- | --- | --- | --- |
| `sidebar_ms` | 0 à 600 | 0 | 120 | 260 | Apparition de la colonne des dossiers |
| `popup_ms` | 0 à 300 | 0 | 80 | 0 | Apparition des menus et sélecteurs (les menus de macOS n'en ont pas) |

### 6.5 `[colors]`

| Clé | Rôle | geek (clair / sombre) | clean | regular |
| --- | --- | --- | --- | --- |
| `background` | Fond de la fenêtre et du lecteur | `#FAFAF8` / `#1B1D23` | `#FFFFFF` / `#1B1D1F` | `system.text_background` |
| `sidebar` | Colonne des dossiers | = `background` | `#F6F6F3` / `#1E2022` | `system.under_page_background` |
| `list` | Liste des mails | = `background` | `#FFFFFF` / `#18191B` | `system.text_background` |
| `bar` | Barre de statut ou barre d'outils | `#EDEDEA` / `#24262E` | `#FFFFFF` / `#18191B` | `system.window_background` |
| `text` | Texte | `#24262B` / `#C3C6C1` | `#1C1E1B` / `#E6E7E3` | `system.label` |
| `muted` | Texte secondaire (dates, compteurs, en-têtes) | `#6B6E75` / `#868B97` (assombri en novembre 2026 : 3,3:1 et 3,6:1 avant, sous WCAG AA) | `#676C65` / `#92978F` | `system.secondary_label` |
| `accent` | Ce qui n'est pas du texte (un cadre) | `#8AA581` | `#8AA581` | `system.accent` |
| `accent_text` | Touches des menus, tags, curseur | `#56704E` / `#8AA581` | `#4E6A46` / `#9DBB93` | `system.accent` |
| `selection` | Ligne choisie | `#D3E0CE` / `#33432F` | `#EEF2EC` / `#272D26` | `system.selected_content_background` |
| `selection_text` | Texte de la ligne choisie | = `text` | = `text` | `#FFFFFF` |
| `marked` | Lignes marquées (`J` `K`) | `#F3E3EE` / `#3A2E45` | idem geek | idem geek |
| `marked_bar` | Barre des lignes marquées | `#C2408F` / `#E08AC0` | idem | idem |
| `separator` | Traits | `#DDDDD8` / `#30333C` | `#ECECE8` / `#2A2C2F` | `system.separator` |
| `popup` | Menus, sélecteurs, panneaux | `#FFFFFF` / `#262833` | `#FFFFFF` / `#222427` | `system.control_background` |
| `unread` | Point des non lus | `#B07D16` / `#E6C07B` | idem geek | `system.accent` |
| `flag` | Drapeau | `#D9661C` / `#F0883E` | idem geek | `system.orange` |
| `error` | Erreurs | `#C0392B` / `#E06C75` | idem | `system.red` |
| `virtual_folder` | Dossiers virtuels (Work, All Mail, Everything) | = `accent_text` | = `accent_text` | = `accent_text` |

**Mails HTML et rédaction.** Le lecteur HTML et l'éditeur des réponses reçoivent les polices et les couleurs résolues ; Dark Reader reçoit `background` et `text` de l'apparence sombre (`ui.md`, « Thème geek : choix retenus »). Le mail lui-même garde ses couleurs quand il prévoit son mode sombre.

**Contraste.** Pour chaque apparence, le noyau calcule le contraste de `text`, `muted` et `accent_text` sur `background` et `list`, et de `selection_text` sur `selection`. Sous 4,5:1 (WCAG AA), c'est un **avertissement**, pas un refus : un thème peu lisible reste le choix de son auteur. Les thèmes livrés n'en ont aucun (test). Une couleur du système n'est pas vérifiée : macOS en répond.

## 7. Ce que fait l'app

- Elle demande le thème choisi au noyau (`theme_get`) et le redemande à chaque `ThemeChanged` ; la fenêtre est redessinée aussitôt, sans relancer et sans perdre la sélection, la lecture ou la réponse en cours.
- Elle traduit les couleurs du système (`system.accent` → `NSColor.controlAccentColor`…), qui suivent ensuite seules l'apparence et les Réglages Système.
- Elle vérifie que chaque police est installée ou livrée. Une police absente est remplacée (section 5.2) et la barre de statut le dit une fois : « Font "Iosevka" not installed: using the system font ».
- Elle embarque les trois thèmes livrés résolus (un JSON produit par le noyau), qui servent de repli tant que le noyau n'a pas répondu et à `FakeCore` dans les tests. Un test du noyau vérifie que ce JSON est à jour.
- Les mesures de la liste (largeur de la date, de l'expéditeur, du nombre de messages) sont faites sur le texte réel dans la police du thème, et non plus en nombre de caractères : n'importe quelle police peut arriver.

## 8. Contrat et CLI

- **`theme_get`** renvoie le thème résolu de `ui.theme` : son nom, sa chaîne d'héritage (`["mon-theme", "clean"]`), toutes les clés du catalogue avec leur valeur (couleurs en paires `light` / `dark` ou en noms du système), et ses avertissements. Un `ui.theme` qui ne désigne aucun thème donne `geek` avec un avertissement ; `config.toml` n'est pas refusé pour autant.
- **`theme_list`** renvoie les thèmes disponibles : nom, origine (`builtin`, `builtin_modified` quand un fichier modifie un thème livré, `user`), parent, et l'erreur qui fait refuser son fichier le cas échéant. Les Préférences s'en servent.
- **`config_set_theme {name}`** écrit `ui.theme` dans `config.toml`, en gardant le reste du fichier tel quel (commentaires et ordre compris). C'est la première écriture de `config.toml` par le noyau en dehors de sa création ; elle se limite à cette clé.
- **Événement `ThemeChanged`**, émis quand le thème appliqué change : un autre `ui.theme`, ou un fichier de sa chaîne d'héritage modifié et valide. Un fichier refusé émet `ConfigError`.
- **CLI** :
  - `nicemailer theme list` : les thèmes, leur origine et leur parent ;
  - `nicemailer theme check [nom | fichier]` : valide sans appliquer, avec le statut 1 si un fichier est refusé, et affiche les avertissements dont ceux de contraste ;
  - `nicemailer theme show [nom] [--json]` : le thème résolu ;
  - `nicemailer theme defaults <nom>` : le fichier complet d'un thème livré, commenté, pour partir d'une copie ;
  - `nicemailer theme new <nom> --extends <parent>` : crée `themes/<nom>.toml` avec `extends` et toutes les clés commentées à la valeur du parent ;
  - `nicemailer config path` liste aussi le dossier `themes/`.

## 9. Choix du thème

- `ui.theme` dans `config.toml` : le nom d'un thème, `"geek"` par défaut.
- Un onglet **Apparence** des Préférences liste les thèmes (`theme_list`), les livrés d'abord ; choisir l'un l'écrit (`config_set_theme`) et l'applique aussitôt. Un thème refusé est grisé, avec son erreur. Un bouton « Ouvrir le dossier des thèmes » ouvre `themes/` dans le Finder.
- Le thème vaut pour toute l'app et toutes ses fenêtres (`ui.md`, « Fenêtres »).
- `ui.title_bar` de `config.toml` est remplacé par `window.title_bar` du thème. Écrit dans `config.toml`, il est encore lu et l'emporte, avec un avertissement qui renvoie au thème ; il sera retiré dans une version suivante.

## 10. Exemple

Un thème personnel, `~/.config/nicemailer/themes/sepia.toml` :

```toml
version = 1
extends = "clean"

[window]
key_hints = true          # garder le rappel des touches de geek
blocks = ["account", "folder", "attributes", "->", "move", "save", "help"]   # sans l'IA ni l'affichage

[fonts]
reading = "Charter"
reading_scale = 1.2

[colors]
background = { light = "#F6F0E4", dark = "#221E18" }
list = { light = "#F6F0E4", dark = "#221E18" }
text = { light = "#3B3025", dark = "#E8DFD0" }
selection = { light = "#EADFC8", dark = "#3A3226" }
```

Et dans `config.toml` :

```toml
[ui]
theme = "sepia"
```

## 11. Tests exigés

- **Lecture** : chaque forme de couleur, de police et de proportion ; chaque cas de refus de la section 3 avec sa ligne et sa clé ; clé inconnue en avertissement ; fichier trop grand.
- **Héritage** : un fichier qui modifie un thème livré ; un thème qui étend un thème du dossier qui étend un thème livré ; `extends` absent ; parent inconnu, cycle, plus de 8 niveaux ; parent refusé (sa dernière version valide sert) ; `blocks` remplacé en entier.
- **Fichiers** : créés commentés au démarrage ; ils donnent exactement les thèmes livrés ; décommenter n'importe quelle ligne donne un fichier valide ; rechargés à chaud ; un thème refusé laisse `config.toml`, `keymap.toml` et les autres thèmes appliqués.
- **Cohérence** : chaque clé du catalogue Rust a son traitement dans l'app et inversement ; les fichiers embarqués reprennent la section 6 ; le JSON embarqué par l'app est à jour ; les thèmes livrés n'ont aucun avertissement de contraste.
- **`config_set_theme`** : il change `ui.theme` et rien d'autre dans le fichier, commentaires compris ; un nom inconnu est refusé.
- **App, tests unitaires** : traduction des couleurs du système ; police absente remplacée et signalée ; mesures de la liste dans une police à chasse fixe (identiques aux largeurs d'aujourd'hui) et proportionnelle ; durées à 0 quand les animations sont réduites ; changement de thème à chaud sur `FakeCore` sans perte de la sélection.
- **App, scénarios UI** : les scénarios existants jouent geek, sans changement ; geek doit rester identique au pixel près après le passage aux fichiers (comparaison des captures avant et après). Un scénario par thème livré (disposition, commande de la fenêtre, menu déroulé depuis son bloc ou son bouton, liste, lecteur, en clair et en sombre). Un scénario par variante du catalogue (6.1, 6.3), sur un thème de test qui l'active. Un scénario qui change de thème sans relancer, un autre avec un thème refusé (erreur affichée, thème précédent gardé). Les fixtures des scénarios gagnent un dossier `fixtures/<scénario>.themes/`, copié dans `themes/` de leur noyau.
