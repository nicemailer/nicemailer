# NiceMailer : raccourcis clavier

Statut : validé et implémenté (novembre 2026 ; `docs/keymap.md`). Complète `configuration.md` (un troisième fichier de configuration) et `ui.md` (les touches qui y sont citées sont celles par défaut).

## 1. Objet

Ce document spécifie comment l'utilisateur redéfinit les touches de l'app : le fichier `keymap.toml`, sa notation, ses contextes, le catalogue des actions et la façon dont l'app s'en sert.

## 2. Principes

- **Pour l'utilisateur avancé, par un fichier.** Il n'y a pas d'interface de modification : l'utilisateur édite `keymap.toml`, que le noyau crée commenté au premier démarrage, et vérifie le résultat avec `nicemailer keymap check`. Le fichier se versionne et se partage avec les dotfiles ; NiceMailer ne le synchronise pas entre Mac.
- **Une touche donne une action.** Le fichier associe des touches à des actions nommées, contexte par contexte (section 5). Une table TOML ne peut pas contenir deux fois la même clé et le parseur le refuse : deux actions sur la même touche d'un même contexte sont impossibles par construction.
- **Le fichier ne contient que les différences.** Une touche absente garde son action par défaut. `"none"` retire une touche par défaut. Il n'existe pas d'option pour partir d'une keymap vide.
- **Une seule source de vérité.** L'aide (`?`), la barre de statut, les menus, la colonne des dossiers, la barre de menus de macOS et les textes d'aide des sélecteurs affichent les touches effectives et non des touches écrites dans le code. Une touche reconfigurée n'est jamais fausse à l'écran.
- **Mêmes touches pour tous les thèmes.** Les thèmes ne sont que des variations graphiques : la keymap ne dépend pas du thème.
- **Le noyau lit le fichier.** Comme pour `config.toml` (`configuration.md`, section 2), c'est le noyau qui lit, valide et surveille le fichier ; l'app reçoit la keymap résolue par le contrat.

## 3. Emplacement, création, lecture

- Le fichier est `keymap.toml`, **dans le dossier de `config.toml`** (`~/.config/nicemailer/`, ou celui désigné par `NICEMAILER_CONFIG`). La même variable déplace ainsi les trois fichiers.
- **Création au démarrage** : le fichier est créé s'il manque, selon les règles de `configuration.md`, section 3. Les en-têtes de tables ne sont pas commentés ; chaque touche par défaut est écrite commentée et suivie de son rôle (`# d = "mail.archive"   # done (archive)`). Il suffit d'enlever le `# ` et de changer l'action. Un fichier existant n'est jamais modifié.
- **Lecture** : au démarrage, puis à chaque changement (FSEvents), sans redémarrage.
- **Indépendant des deux autres fichiers** : une keymap invalide ne fait pas refuser `config.toml` ni `accounts.toml`, et inversement. Une faute dans un raccourci ne doit pas désactiver les comptes.
- **Fichier invalide : refusé en entier.** Le noyau garde la keymap précédente et émet `ConfigError {file: "keymap.toml", line, message}`. L'app l'affiche en rouge dans la barre de statut, avec la ligne, jusqu'à ce que le fichier soit corrigé (il n'y a pas encore de bandeau pour les fichiers de configuration). Au démarrage, sans keymap précédente, ce sont les valeurs par défaut. Sont invalides :
  - une syntaxe TOML incorrecte ;
  - une touche mal écrite ;
  - un contexte ou un menu inconnu ;
  - une action interdite dans son contexte (section 6) ;
  - une touche réservée (section 4.4) ;
  - une touche de caractère, ou une touche nécessaire à l'éditeur, dans un contexte de saisie (section 5.2) ;
  - deux écritures d'une même touche dans une table (`J` et `shift-j`) ;
  - un menu vide ou inaccessible quand il est appelé.
- **Action inconnue : avertissement**, pas erreur. La ligne est ignorée et `keymap check` la signale. Une keymap écrite pour une version plus récente de NiceMailer reste ainsi lisible par une plus ancienne, comme une clé inconnue de `config.toml`.
- **Version du format** : `version = 1`.

## 4. Notation des touches

### 4.1 Caractères

Une touche sans ⌃, ⌥ ni ⌘ s'écrit **par le caractère qu'elle produit** : `j`, `J`, `?`, `*`, `@`, `;`, `/`. `J` implique Shift ; `shift-j` est accepté et vaut `J`. Le caractère produit dépend de la disposition du clavier, et c'est voulu : `?` est la touche `?`, en QWERTY comme en AZERTY. Une clé TOML qui n'est pas une lettre ou un chiffre se met entre guillemets : `"?" = "help.show"`.

### 4.2 Touches nommées

`space`, `enter`, `tab`, `esc`, `backspace` (⌫), `del` (⌦), `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1` à `f12`. Pour une touche nommée, Shift s'écrit explicitement : `shift-space`, `shift-down`, `shift-tab`.

### 4.3 Modificateurs

`ctrl-`, `alt-` (⌥), `shift-`, `cmd-`, dans n'importe quel ordre : `cmd-shift-a` et `shift-cmd-a` sont la même touche. Avec ⌃, ⌥ ou ⌘, la touche est **celle qu'on lit sur le clavier**, en minuscule, et non le caractère produit. ⌥ produit des caractères sur Mac (⌥n donne `~` en AZERTY) et ⌃ des caractères de contrôle. Si l'app ne peut pas lire la lettre (⌃ donne 0x06 pour F), elle se replie sur la position de la touche, comme ⌃⌘F aujourd'hui. `ctrl-C` est accepté et vaut `ctrl-shift-c`.

L'affichage (aide, menus, barre de statut) utilise les symboles du Mac dans l'ordre du Mac, ⌃⌥⇧⌘ : `ctrl-shift-c` s'affiche `⌃⇧C`.

### 4.4 Touches réservées

`esc` et `cmd-q` ne peuvent être ni liés ni retirés, dans aucun contexte. `esc` doit toujours permettre de revenir en arrière (fermer, annuler, démarquer), et ⌘Q de quitter. Les autres raccourcis de macOS (⌘, ⌘H ⌘M ⌘W) restent possibles : celui qui les lie sait ce qu'il fait.

### 4.5 Pas de séquences

Il n'y a pas de séquences de touches libres (`g g`) ni de délai d'attente. Une séquence passe par un menu (section 7) : la première touche ouvre le menu, la seconde y choisit une ligne. Une touche est donc soit une action, soit l'ouverture d'un menu, jamais les deux. Aucune ambiguïté n'est possible.

## 5. Contextes

### 5.1 Liste et priorité

Une touche est cherchée dans le contexte le plus précis actif, puis dans ses parents, jusqu'à `everywhere`.

| Contexte | Actif quand | Parent |
| --- | --- | --- |
| `everywhere` | toujours, même pendant l'écriture d'une réponse ou dans un panneau | — |
| `normal` | aucun panneau ouvert, pas de réponse en cours, quel que soit l'écran | `everywhere` |
| `mail` | écran des mails (liste et lecteur) | `normal` |
| `quarantine` | un dossier Quarantine est listé (pas un tag, un groupe, un contact) | `mail` |
| `screen` | écrans Contacts, Historique, Tags, Groupes | `normal` |
| `contacts`, `tags`, `groups` | l'écran du même nom | `screen` |
| `menu_navigation` | un menu est ouvert (section 7) | `everywhere` |
| `picker` | un sélecteur est ouvert (`@ f / l t g`, renommage) | `line_edit` |
| `contact_filter` | la ligne de filtre de Contacts est ouverte | `line_edit` |
| `line_edit` | une ligne de texte a les touches (sélecteur, filtre, sujet, destinataires) | `everywhere` |
| `choice` | une question attend sa réponse | `everywhere` |
| `ai_panel` | le panneau IA d'un mail ou d'une réponse est ouvert | `everywhere` |
| `compose` | une réponse est en cours d'écriture | `everywhere` |

Un `"none"` dans un contexte masque aussi l'action de ses parents pour cette touche : la touche est alors sans effet dans ce contexte.

Pendant l'écriture d'une réponse, une ligne (le sujet, une ligne de destinataires) cherche dans `line_edit`, puis `compose`, puis `everywhere` : ⌘V y colle sur une ligne, et ⌘↩ envoie toujours. Cette chaîne est celle de l'app ; la colonne « Parent » du tableau sert à la validation (où une action peut être liée).

### 5.2 Contextes de saisie

Dans `everywhere`, `compose`, `line_edit`, `picker` et `contact_filter`, une touche qui produit un caractère sans ⌃ ni ⌘ (une lettre, un symbole, `space`, ⌥ avec un caractère) est **refusée**, car ce caractère ne pourrait plus être tapé. Sont aussi refusées les touches dont le texte a besoin : dans `everywhere` et `compose`, `enter`, `backspace`, `del`, les flèches, `home`, `end`, `pageup`, `pagedown` sans ⌃ ⌥ ⌘ (et `tab` dans `everywhere` ; dans `compose`, Tab passe d'un champ à l'autre hors du texte) ; dans une ligne, ← → ⌫ ⌦ (section 5.3).

Dans `line_edit`, Shift ajouté à une touche de déplacement étend la sélection. C'est une règle et non une liaison : `alt-left` lié à `line.word_left` donne aussi `alt-shift-left`, qui étend la sélection d'un mot.

### 5.3 Ce qui n'est pas configurable

- les touches de l'éditeur du texte d'une réponse (Tiptap, `edition.md`), qui reçoit toutes les touches que `compose` ne prend pas ;
- les caractères tapés dans une ligne de texte, ainsi que ←, →, ⌫ et ⌦ simples dans une ligne (les déplacements et effacements de base d'un champ macOS) ;
- dans une réponse, le comportement propre aux champs : entrée, `,` et `;` qui valident une adresse, ⌫ qui reprend la dernière adresse, ← → ⌫ dans la ligne des pièces jointes, entrée dans le sujet ;
- les lettres des réponses d'une question (`choice`), affichées avec la question, et les chiffres qui choisissent une ligne dans le panneau IA ;
- `esc` et `cmd-q` (section 4.4) ;
- la souris.

## 6. Actions

Une action a un nom avec un espace de noms (`mail.archive`). Le catalogue fixe pour chaque action les contextes où elle peut être liée ; ailleurs, la lier est une erreur (`compose.send` n'a pas de sens dans `contacts`). Le catalogue fixe aussi son libellé (traduit, dans l'aide) et son groupe dans l'aide.

### 6.1 Catalogue et valeurs par défaut

Ce tableau est l'inventaire des touches d'aujourd'hui. Il devient les valeurs par défaut, et l'implémentation doit d'abord les reproduire exactement.

**`everywhere`**

| Touches | Action | Rôle |
| --- | --- | --- |
| `ctrl-cmd-f` | `window.full_screen` | plein écran, et retour |
| `cmd-+` `cmd-=` | `text.bigger` | texte plus grand (`cmd-=` : `+` demande Shift en AZERTY) |
| `cmd--` | `text.smaller` | texte plus petit |
| `cmd-0` | `text.reset` | taille par défaut |
| `cmd-r` | `app.reload` | recharger (menu Présentation) |

**`normal`**

| Touches | Action | Rôle |
| --- | --- | --- |
| `;` | `menu.goto` | menu des dossiers et des écrans |
| `?` | `help.show` | liste des touches |
| `z` | `undo.last` | annuler le dernier déplacement ou la dernière décision |

**`mail`**

| Touches | Action | Rôle |
| --- | --- | --- |
| `j` `down` | `list.down` | mail suivant |
| `k` `up` | `list.up` | mail précédent |
| `J` `shift-down` | `list.mark_down` | marquer et descendre |
| `K` `shift-up` | `list.mark_up` | marquer et monter |
| `cmd-a` | `list.mark_all` | marquer toute la liste |
| `ctrl-d` | `list.half_page_down` | demi-page vers le bas |
| `ctrl-u` | `list.half_page_up` | demi-page vers le haut |
| `ctrl-j` | `message.next` | message suivant de la conversation |
| `ctrl-k` | `message.previous` | message précédent de la conversation |
| `enter` | `mail.open` | suggestion de l'IA acceptée, brouillon rouvert, ou mail en pleine fenêtre (une colonne) |
| `space` | `reader.open_or_page_down` | une colonne, sur la liste : ouvrir ; sinon page suivante |
| `ctrl-f` | `reader.page_down` | page suivante du mail |
| `shift-space` `ctrl-b` | `reader.page_up` | page précédente du mail |
| `T` | `reader.toggle_all` | citations, signatures, mails repliés |
| `S` | `reader.skip_quoted` | aller au texte après la citation suivante |
| `l` | `picker.links` | liens du mail |
| `ctrl-c` | `sender.card_or_copy` | fiche Contacts, ou adresse copiée |
| `ctrl-shift-c` | `sender.mails` | mails de l'expéditeur |
| `d` | `mail.archive` | fait (archiver) |
| `m` | `menu.move` | menu de déplacement |
| `backspace` `del` | `mail.delete` | supprimer |
| `u` | `mail.toggle_seen` | lu / non lu |
| `U` | `mail.mark_seen` | marquer lu |
| `*` | `mail.toggle_flag` | drapeau |
| `t` | `picker.tag` | ajouter / retirer un tag |
| `g` | `picker.group` | ajouter / retirer un groupe |
| `s` | `menu.save` | menu d'enregistrement |
| `cmd-u` | `mail.unsubscribe` | se désabonner de la liste |
| `r` | `compose.reply_all` | répondre à tous |
| `R` | `compose.reply_sender` | répondre à l'expéditeur seul |
| `F` | `compose.forward` | transférer |
| `c` | `compose.new` | nouveau mail |
| `a` | `ai.menu` | menu IA du mail ou de la liste |
| `/` | `picker.search` | chercher partout |
| `f` | `picker.attributes` | filtrer par attributs |
| `@` | `picker.accounts` | aller à un compte |
| `v` | `menu.view` | menu d'affichage |

`esc` (`mail.back`) garde sa cascade (`ui.md`) et n'est pas configurable.

**`quarantine`** : les cibles en une touche, sans `m` (`ui.md`, « Quarantine »).

| Touches | Action |
| --- | --- |
| `i` | `move.inbox` |
| `n` | `move.newsletter` |
| `p` | `move.paperwork` |
| `x` | `move.junk` |
| `b` | `move.banned` |
| `d` | `move.deleted` |

Les actions refusées en Quarantine sont une propriété du catalogue, **pas des touches** : `mail.toggle_seen`, `mail.mark_seen`, `picker.tag`, `picker.group`, `compose.reply_all`, `compose.reply_sender`, `compose.forward` et `menu.save` y disent pourquoi elles ne font rien, quelle que soit leur touche. `menu.move` y rappelle les touches des cibles.

**`screen`**

| Touches | Action |
| --- | --- |
| `j` `down` | `screen.down` |
| `k` `up` | `screen.up` |
| `enter` `l` | `screen.open` (mails du contact, du tag ou du groupe) |

**`contacts`** : `/` `contacts.filter`, `m` `contacts.decide_address`, `M` `contacts.decide_domain`, `o` `contacts.sender_kind`.

**`tags`, `groups`** : `r` `label.rename`, `d` `label.delete`.

**`menu_navigation`** : `down` `menu.next`, `up` `menu.previous`, `enter` `menu.choose`, `cmd-enter` `menu.choose_alt` (seconde action d'une ligne, par exemple enregistrer puis ouvrir).

**`picker`** : `down` `ctrl-n` `ctrl-j` `picker.next`, `up` `ctrl-p` `ctrl-k` `picker.previous`, `enter` `picker.choose`, `cmd-enter` `picker.choose_alt`.

**`contact_filter`** : `enter` `contact_filter.close` (ferme la ligne et garde le filtre).

**`line_edit`** : `ctrl-a` `cmd-left` `home` `line.start`, `ctrl-e` `cmd-right` `end` `line.end`, `alt-left` `line.word_left`, `alt-right` `line.word_right`, `alt-backspace` `line.delete_word`, `cmd-backspace` `line.delete_to_start`, `cmd-a` `line.select_all`, `cmd-c` `line.copy`, `cmd-x` `line.cut`, `cmd-v` `line.paste`.

**`choice`** : `down` `choice.next`, `up` `choice.previous`, `enter` `choice.choose`.

**`ai_panel`** : `y` `ai.accept`, `n` `ai.refuse`, `c` `ai.copy`, `enter` `ai.confirm`, `cmd-enter` `ai.open`, `down` `ai.next`, `up` `ai.previous`.

**`compose`**

| Touches | Action | Rôle |
| --- | --- | --- |
| `cmd-enter` | `compose.send` | envoyer |
| `cmd-shift-enter` | `compose.file_and_send` | choisir où va la conversation, puis envoyer |
| `cmd-esc` | `compose.abandon` | abandonner le brouillon |
| `ctrl-i` | `ai.compose_menu` | menu IA de la réponse |
| `cmd-shift-t` | `compose.toggle_plain` | texte seul ↔ mis en forme |
| `cmd-shift-a` | `compose.attach` | joindre des fichiers |
| `cmd-alt-f` | `compose.from` | adresse et compte d'envoi |
| `cmd-alt-b` | `compose.bcc` | ligne Cci |
| `tab` | `compose.next_field` | champ suivant (hors texte) |
| `shift-tab` | `compose.previous_field` | champ précédent (hors texte) |
| `cmd-v` | `compose.paste` | fichiers du Finder joints, sinon collage du texte |
| `cmd-alt-shift-v` | `compose.paste_plain` | coller le texte seul |

Les lignes des tableaux de cette section et le fichier des valeurs par défaut doivent rester identiques. Un test le vérifie (section 11).

## 7. Menus

Un menu est une table `[menu.<nom>]` : ses clés sont les secondes touches, ses valeurs des actions. Une action `menu.<nom>` l'ouvre, depuis n'importe quelle touche. Le menu affiche ses lignes avec le libellé de chaque action, dans l'ordre du fichier des valeurs par défaut, puis dans celui du fichier de l'utilisateur.

Menus fournis :

- **`menu.move`** : `i` `move.inbox`, `t` `move.todo`, `w` `move.waiting`, `a` `move.archive`, `n` `move.newsletter`, `p` `move.paperwork`, `q` `move.quarantine`, `d` `move.deleted`, `x` `move.junk`, `b` `move.banned`. Le menu ne montre que les cibles permises depuis le dossier du mail (`move_targets`, `dossiers.md`). Une cible permise sans touche n'apparaît pas.
- **`menu.goto`** :
  - les dossiers : `i` `goto.inbox`, `t` `goto.todo`, `w` `goto.waiting`, `o` `goto.work`, `a` `goto.archive`, `n` `goto.newsletter`, `p` `goto.paperwork`, `s` `goto.sent`, `r` `goto.drafts`, `m` `goto.all_mail`, `q` `goto.quarantine`, `d` `goto.deleted`, `x` `goto.junk`, `b` `goto.banned`, `e` `goto.everything` ;
  - les écrans : `c` `goto.contacts`, `T` `goto.tags`, `g` `goto.groups`, `h` `goto.history`.

  Le menu garde ses blocs (`ui.md`). Un dossier absent de la vue n'apparaît pas. La colonne des dossiers affiche la touche de `goto.<rôle>`.
- **`menu.save`** : `c` `save.clipboard`, `e` `save.eml`, `t` `save.text`, `T` `save.text_open`, `m` `save.markdown`, `M` `save.markdown_open`, `a` `save.attachments`, `g` `save.attachments_no_graphics`, `p` `save.pdf`, `P` `save.pdf_open`, `i` `save.pdf_evidence`, `I` `save.pdf_evidence_open`. Une action et sa variante `_open` forment une ligne : la variante est le `menu.choose_alt` de la ligne, et sa touche s'affiche à droite (« T : et ouvrir »).
- **`menu.view`** : `1` `view.one_column`, `2` `view.two_columns`, `3` `view.three_columns`, `c` `view.sort_received`, `m` `view.sort_modified`, `h` `view.toggle_html`, `i` `view.toggle_remote_images`, `s` `view.cycle_status_bar`.

Les actions `move.*` et `goto.*` peuvent aussi être liées directement, hors menu : `I = "goto.inbox"` dans `[mail]`.

**Menus de l'utilisateur** : une table `[menu.<nom>]` dont le nom n'est pas fourni crée un menu. Il s'ouvre par `menu.<nom>`, n'a pas de bloc dans la barre de statut, s'affiche comme les autres menus avec `<nom>` pour titre, et apparaît dans l'aide sous la touche qui l'ouvre. Un menu de l'utilisateur sans ligne, ou qu'aucune touche n'ouvre, est une erreur. Dans un menu, une action `menu.*` n'est pas permise : il n'y a pas de menus imbriqués.

**Ouverture masquée** : lier ailleurs la touche qui ouvrait un menu fourni (`m = "mail.archive"`) sans redonner une touche à `menu.move` laisse le menu inaccessible au clavier. C'est un **avertissement** et non une erreur : le bloc de la barre de statut l'ouvre encore au clic.

## 8. Exemple

```toml
version = 1

[mail]
e = "mail.archive"       # archive with e, as in Gmail
d = "none"               # d no longer archives
"#" = "mail.delete"
I = "goto.inbox"         # straight to the Inbox, without `;`
space = "menu.mine"

[menu.mine]
a = "save.attachments"
p = "save.pdf_open"

[compose]
"cmd-shift-s" = "compose.file_and_send"
```

## 9. Contrat et CLI

- **`keymap_get`** renvoie la keymap résolue : pour chaque contexte et chaque menu, la liste `{touche, action}` en notation canonique (valeurs par défaut et fichier fusionnés, `"none"` appliqués), plus les avertissements.
- **Événement `KeymapChanged`**, émis quand une nouvelle keymap valide est appliquée. Un fichier refusé émet `ConfigError` (section 3).
- **CLI** :
  - `nicemailer keymap defaults` : le fichier complet des valeurs par défaut, commenté ;
  - `nicemailer keymap check [fichier]` : valide sans appliquer, avec le statut 1 si le fichier est refusé et les avertissements affichés ;
  - `nicemailer keymap show [--json]` : la keymap effective, par contexte ; avec `--defaults`, les valeurs par défaut seules (`--defaults --json` écrit le JSON embarqué par l'app) ;
  - `nicemailer config path` liste aussi `keymap.toml`.

L'app embarque les valeurs par défaut résolues (un JSON produit par le noyau). Elles servent à `FakeCore` dans les tests et de repli tant que le noyau n'a pas répondu. Un test du noyau vérifie que ce JSON est à jour.

## 10. Affichage des touches

Tout ce qui montre une touche la prend dans la keymap effective :

- l'aide `?`, dont les groupes et les libellés viennent du catalogue. Une action à plusieurs touches les montre toutes ; une action sans touche n'apparaît pas ;
- les blocs de la barre de statut (`ui.md`, « Thème geek ») et les cibles de Quarantine ;
- les lignes des menus et leurs indications (« T : et ouvrir ») ;
- la colonne des dossiers ;
- la barre de menus de macOS (Présentation : Recharger, taille du texte) ;
- les textes d'aide des sélecteurs, des écrans, du panneau IA et de la réponse qui citent une touche ; une action sans touche y est nommée par son nom (`compose.toggle_plain`) ;
- le message « Pas de commande x ».

Les touches citées dans `ui.md` et `docs/` sont celles par défaut ; `docs/keymap.md` le rappelle.

## 11. Tests exigés

- **Notation** : lecture et affichage aller-retour de chaque forme ; équivalences (`shift-j` et `J`, `ctrl-C` et `ctrl-shift-c`, ordre des modificateurs) ; refus des formes invalides avec leur ligne.
- **Conversion d'un événement clavier** : QWERTY et AZERTY, caractères de contrôle avec ⌃, ⌥ qui produit un caractère, Shift sur `space` et sur les flèches.
- **Résolution** : priorité des contextes, `"none"` qui masque un parent, menus de l'utilisateur, avertissement d'ouverture masquée.
- **Refus** : chaque cas de la section 3, dont `esc` et `cmd-q`, une touche de caractère dans un contexte de saisie, une action hors de ses contextes, un menu vide ; la keymap précédente est gardée et `config.toml` reste appliqué.
- **Fichier** : créé commenté au démarrage ; il donne exactement les valeurs par défaut ; décommenter n'importe quelle ligne donne un fichier valide ; il est rechargé à chaud.
- **Cohérence** : chaque action du catalogue Rust a son traitement dans l'app et inversement ; le fichier des valeurs par défaut reprend la section 6.1 ; le JSON embarqué est à jour.
- **Sans changement de comportement** : tous les tests et scénarios UI existants passent avec les valeurs par défaut.
- **App** : un scénario UI avec une keymap de test (`e` archive, `d` retiré, une touche dans un menu de l'utilisateur) vérifie l'action, l'aide et la barre de statut.
