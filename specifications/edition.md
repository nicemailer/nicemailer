# NiceMailer : édition des mails

Statut : réalisé (novembre 2026, `docs/editor.md`), étapes 1 à 3 de la section 13 ensemble. Écarts : la confirmation du passage au texte seul est un second `⇧⌘T` (section 3) ; le texte brut a sa propre conversion (section 10) ; l'IA refuse un brouillon mis en forme (section 14) ; le CLI et l'agent MCP n'ont pas encore de commande d'envoi (section 11). Remplace, pour la zone de texte, les règles de `reponse.md` (section 6 : « le texte est édité par l'app elle-même, pas par un champ AppKit » ; section 7 : format du mail) et complète `brouillons.md` (table `draft`, question « Format de rédaction »), `ui.md` (contexte « edition ») et `architecture.md` (principe 3, section 2 ci-dessous).

## 1. Objet

Ce document spécifie le composant qui édite le texte d'un mail, ses deux modes (texte seul, mis en forme), ses raccourcis, le collage, le format stocké et ce que le noyau en fait à l'envoi.

## 2. Principes

- **Deux interfaces dans l'app.** La navigation (l'essentiel du temps) est faite de touches sans modificateur, propres à NiceMailer. L'écriture d'un mail suit **le Mac** : ses raccourcis, ses intégrations, son comportement de texte. Les commandes de l'app y portent un modificateur et sont en nombre fermé (section 7).
- **Un composant d'édition standard : Tiptap**, éditeur web bâti sur ProseMirror, dans une `WKWebView`. Il est structuré par un schéma (rien n'entre dans le texte qui n'y figure pas), il comprend nativement le HTML, et il n'est pas lié à macOS : le même composant servira l'interface web (`nm-web`, `architecture.md`, section 3) et un éventuel client sur une autre plateforme.
- **Les champs techniques restent ceux de l'app.** De, À, Cc, Cci et l'objet gardent leurs composants (`LineEdit`, étiquettes, suggestions, `reponse.md`, section 6). Seul le texte du mail passe dans l'éditeur.
- **Le HTML est le format de la mise en forme.** Ce que Tiptap produit est stocké tel quel dans le brouillon ; un seul analyseur, celui de l'éditeur, transforme un HTML (collé, ou d'un brouillon rouvert) en document conforme au schéma.
- **Le noyau décide de ce qui part.** À l'envoi, il applique au HTML une liste fermée de balises, d'attributs et de valeurs (section 9), quoi que l'éditeur ait produit. Un éditeur bogué ne peut rien envoyer d'autre.
- **Pas de Markdown.** NiceMailer ne lit ni n'écrit de Markdown. Un texte rédigé en Markdown ailleurs (Obsidian) se colle **en HTML**, copié comme tel par l'app d'origine. Les raccourcis de frappe de Tiptap (`# `, `- `, `**x**`, section 6) sont gardés pour leur confort : ils mettent en forme à la frappe, aucun Markdown n'est stocké.

**Principe 3 d'`architecture.md` précisé.** « Aucune logique métier côté Swift » reste vrai. La logique d'édition (schéma, collage, listes) vit dans le composant d'édition, qui est commun à toutes les interfaces graphiques ; la règle de ce qui est envoyé, le texte brut et le MIME restent dans le noyau. `architecture.md` est mis à jour en ce sens.

## 3. Les deux modes

| | Texte seul | Mis en forme |
| --- | --- | --- |
| Schéma de l'éditeur | Paragraphes et texte | Section 5 |
| Barre de mise en forme | Absente | Toujours affichée (section 6) |
| Raccourcis de frappe, ⌘B, ⌘I, ⌘U, ⌘K | Sans effet | Actifs |
| `draft.body_format` | `text` | `html` |
| `draft.body` | Le texte, lignes séparées par `\n` | Le HTML de Tiptap |
| Mail envoyé | `text/plain` (`format=flowed`), comme aujourd'hui | Section 10 |

- **Mode par défaut : texte seul**, pour un nouveau mail, une réponse et un transfert. Le réglage `compose.format` (`text` ou `html`, `text` par défaut) le change.
- **Changer de mode** : `⇧⌘T`, comme « Convertir au format Texte / Convertir au format RTF » dans Mail. Le mode part avec la pause de frappe suivante (`update_draft` avec `body_format`).
- **Du texte seul vers mis en forme** : chaque ligne devient un paragraphe ; les lignes qui commencent par `>` deviennent une citation, `>>` une citation imbriquée (le préfixe est retiré). Rien n'est perdu.
- **Du mis en forme vers texte seul** : le texte est celui que produit le noyau pour la partie `text/plain` (section 10), donc les titres, listes et citations restent lisibles ; les styles sont perdus. Si le texte a une mise en forme, un premier `⇧⌘T` le dit dans la barre d'état (« La mise en forme sera perdue · ⇧⌘T de nouveau pour convertir ») et un second, dans les cinq secondes, convertit. Sans mise en forme, la conversion est immédiate. (Réalisé ainsi plutôt qu'avec une question `ASK` : le même geste confirme, sans quitter le texte.)
- **Conversion en un seul endroit.** Les deux conversions sont faites par l'éditeur (texte vers document) et par le noyau (HTML vers texte, la même fonction que pour l'envoi). Le composant ne contient pas de seconde conversion HTML vers texte.

## 4. Le composant

### Emplacement et outillage

- Le code de l'éditeur est dans `apps/editor/`, en **TypeScript**, sans aucune dépendance à macOS ni à WebKit. Il ne connaît que le protocole de la section 8.
- Dépendances : `@tiptap/core`, `@tiptap/pm` et les extensions libres (licence MIT) de la section 5. Aucune extension payante. Versions fixées exactement, fichier de verrouillage commité, `npm ci`.
- Node : version fixée dans le dépôt (`.nvmrc` et `engines` de `package.json`).
- **Bundle construit, pas commité.** `make editor` produit un seul fichier (esbuild) dans les ressources de l'app ; `make` (et donc la construction de l'app) en dépend, et le fichier produit est ignoré par git. Construire l'app demande donc Node. Le bundle correspond toujours au code source.
- Les licences des dépendances sont embarquées dans l'app, comme pour Dark Reader.
- Commentaires en anglais, avec le même style pédagogique que le Rust et le Swift (« TypeScript note: ») ; `CLAUDE.md` est complété en ce sens.

### Hôte macOS

- Une `WKWebView` dédiée (`EditorWebView`), **préchargée dès la première liste affichée** (pas pendant le lancement, qu'elle ralentissait) et gardée cachée ; les touches tapées pendant que sa page se charge encore sont gardées par l'app et tapées dès qu'elle est prête : `r`, `R`, `F`, `c` ouvrent l'éditeur sans attendre le chargement de WebKit. Le contenu est remplacé à chaque ouverture (message `load`).
- **Isolement** : bundle chargé depuis les ressources de l'app ; politique de contenu `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'` ; une règle de contenu WebKit qui bloque tout chargement réseau ; aucune navigation (un clic sur un lien ne l'ouvre pas) ; stockage de données non persistant ; pas d'accès aux fichiers hors du bundle.
- **Apparence** : police, taille du texte (⌘+ ⌘− de l'app), couleurs du thème clair ou sombre, passées par variables CSS dans `load` et `theme`. ⌘+ et ⌘− restent ceux de l'app, jamais le zoom de WebKit.
- **Textes de l'interface de l'éditeur** (barre, boîte de lien) : fournis par Swift dans `load`, depuis le String Catalog de l'app. Il n'y a qu'une source de traduction.
- **Fichiers glissés** sur l'éditeur : interceptés par la vue native et ajoutés comme pièces jointes, par le circuit actuel (`incoming.noindex`, `add_draft_attachment`, `brouillons.md`, section 7). Ils ne deviennent jamais des images dans le texte.

## 5. Le schéma (mode mis en forme, v1)

| Élément | Extension Tiptap | Règle |
| --- | --- | --- |
| Paragraphe, retour à la ligne (`⇧↩`) | `Paragraph`, `HardBreak` | |
| Titres | `Heading` | Trois niveaux : 1, 2, 3 |
| Listes à puces, numérotées | `BulletList`, `OrderedList`, `ListItem` | Imbriquées |
| Citation | `Blockquote` | Imbriquée ; sert d'abord au texte cité d'une réponse |
| Gras, italique, souligné | `Bold`, `Italic`, `Underline` | |
| Lien | `Link` | `http`, `https`, `mailto` seulement ; lien automatique à la frappe d'une URL ; pas d'ouverture au clic |
| Couleur du texte | `TextStyle`, `Color` | Palette fermée (ci-dessous) |
| Police | `FontFamily` | Toutes les polices du Mac, dont le nom est sûr (ci-dessous) |
| Taille | `FontSize` | Liste fermée (ci-dessous) |
| Annulation | `UndoRedo` | |

**Exclus** : tableaux, images, code et bloc de code, texte barré, ligne horizontale, alignement, couleur de fond. Ceux qu'apporte `StarterKit` sont désactivés, avec leurs raccourcis de frappe.

**Polices** : « Par défaut » (aucune police indiquée : le client du destinataire choisit, ce qui est le plus sûr), puis une partie « Utilisées récemment » (les cinq dernières choisies, mémorisées par l'app), puis toutes les polices du Mac. Chacune part avec un remplacement, pour qu'un destinataire qui ne l'a pas lise le même genre de police : les polices présentes sur Windows, macOS, iOS et Android (Arial, Helvetica, Verdana, Tahoma, Trebuchet MS, Georgia, Times New Roman, Courier New) avec la pile du noyau (`Georgia, 'Times New Roman', serif`), les autres avec leur famille générique, lue dans les traits de la police (`'Avenir Next', sans-serif`). Le noyau ne peut pas connaître les polices du Mac : il garde toute police dont le nom est sûr en CSS (lettres, chiffres, espaces, `- . _ &`). (D'abord une liste fermée de huit polices, jugée trop restrictive à l'usage.)

**Tailles** : Normale (aucune taille indiquée), puis les tailles du menu d'Apple Mail : 9, 10, 11, 12, 13, 14, 18, 24, 36, 48, 64, 72, 96, 144, 288 px. En pixels, l'unité que rendent tous les clients, Outlook compris.

**Couleurs** : huit teintes de luminance moyenne (gris, rouge, orange, vert, bleu, violet, et deux à fixer avec le design), lisibles sur fond blanc **et** sur fond sombre : le destinataire peut lire en thème sombre, et son client n'inverse pas toujours les couleurs choisies. Pas de noir ni de blanc : « Par défaut » laisse la couleur du destinataire.

**Valeurs hors liste** (un collage, un brouillon venu d'ailleurs) : la police, la taille ou la couleur est retirée, le texte reste. Un `parseHTML` propre à chaque extension n'accepte que les valeurs de la liste.

## 6. Mise en forme

- **Raccourcis de frappe** (gardés) : `# `, `## `, `### ` en tête de ligne font un titre ; `- ` ou `* ` une liste à puces ; `1. ` une liste numérotée ; `> ` une citation ; `**x**` du gras, `*x*` de l'italique. `⌘Z` juste après en annule l'effet et rend le texte tapé, comme dans Tiptap.
- **Raccourcis clavier** : ceux de Tiptap, qui sont ceux du Mac : `⌘B`, `⌘I`, `⌘U`, `⌘K` (lien), `⌥⌘1` à `⌥⌘3` (titres), `⇧⌘7` (liste numérotée), `⇧⌘8` (liste à puces), `⇧⌘B` (citation).
- **Listes** : `↩` sur un élément vide termine la liste, `⇥` et `⇧⇥` indentent et désindentent un élément.
- **Barre de mise en forme**, dans la vue web (donc commune à toutes les interfaces) : police, taille, couleur, gras, italique, souligné, titres, listes, citation, lien. Elle reflète la mise en forme sous le curseur. Elle est toujours affichée en mode mis en forme, et jamais en texte seul : `⇧⌘T` est la seule bascule (un bouton pour la masquer a été retiré : un texte mis en forme sans sa barre cache ce qu'il permet). Ses commandes s'atteignent aussi au clavier, par les raccourcis ci-dessus.
- **Boîte de lien** (`⌘K`) : texte et adresse, dans l'éditeur ; refuse un schéma hors liste.

## 7. Touches dans la zone de texte

L'app reçoit chaque touche avant l'éditeur (moniteur de touches de `GeekController`). Elle ne garde que la liste fermée ci-dessous ; toutes les autres vont à l'éditeur, qui suit le comportement du Mac.

| Touche | Effet | Changement |
| --- | --- | --- |
| `⌘↩` | Envoyer | Inchangé ; attend d'abord le dernier contenu (section 8) |
| `⇧⌘↩` | Choisir le rangement, puis envoyer | Inchangé |
| `⌥⌘F` | Adresse d'envoi | Inchangé |
| `⌥⌘B` | Ligne Cci | Inchangé |
| `⇧⌘A` | Joindre des fichiers | Inchangé |
| `⌃I` | Palette IA | **Était `⌘I`**, rendu à l'italique |
| `⌘⎋` | Abandonner le brouillon | **Était `⌘⌫`**, rendu à « effacer jusqu'au début de la ligne ». Pas `⌥⎋` : c'est le raccourci par défaut de macOS pour « Énoncer la sélection » |
| `⎋` | Revenir au mail, garder le brouillon | Inchangé ; laissé à l'éditeur quand une boîte de l'éditeur est ouverte (lien, menu de la barre), qu'il signale par `state` |
| `⇧⌘T` | Texte seul / mis en forme | Nouveau (section 3) |
| `⌥⇧⌘V` | Coller sans mise en forme | Nouveau (section 9) |
| `⌘+`, `⌘−`, `⌘0` | Taille du texte de l'app | Inchangé, jamais le zoom de WebKit |

- **`⇥` et `⇧⇥`** : seul l'éditeur sait si le curseur est dans une liste. Dans une liste, il indente ou désindente. Ailleurs, `⇧⇥` envoie `leave` (retour à l'objet) ; `⇥` insère une tabulation en texte seul et ne fait rien en mode mis en forme (une tabulation ne se rend pas en HTML).
- **Annulation** : `⌘Z` et `⇧⌘Z`, ainsi que les éléments « Annuler » et « Rétablir » du menu Édition, vont à l'historique de l'éditeur.
- La documentation (`ui.md`, `docs/geek-ui.md`, `docs/sending.md`, aide de l'app) est mise à jour pour `⌃I` et `⌘⎋`.

## 8. Protocole entre l'hôte et l'éditeur

Un seul canal de messages JSON, dans les deux sens, versionné (`v` dans `ready` et `load`). Il est décrit dans `apps/editor/PROTOCOL.md`, la référence pour tout hôte (app macOS, interface web). Aucun message ne suppose WebKit.

**Hôte vers éditeur**

| Message | Contenu |
| --- | --- |
| `load` | Mode, contenu (texte ou HTML), position du curseur, thème, taille du texte, langue et textes de l'interface, polices du Mac et polices récentes |
| `theme` | Thème et taille du texte, quand ils changent |
| `replaceSelection` | Texte ou HTML à mettre à la place de la sélection (IA, collage sans mise en forme), en une étape d'annulation |
| `setMode` | Changement de mode, avec le contenu converti |
| `flush` | Demande du contenu courant, avec un identifiant |
| `focus` | Donner le curseur à l'éditeur |

**Éditeur vers hôte**

| Message | Contenu |
| --- | --- |
| `ready` | Version du protocole |
| `changed` | Contenu et révision locale, envoyé à chaque pause de frappe (0,5 s, comme aujourd'hui) |
| `flushed` | Le contenu courant, en réponse à `flush` |
| `state` | Mise en forme sous le curseur, une boîte ouverte ou non, sélection vide ou non, texte de la sélection |
| `leave` | `⇧⇥` hors d'une liste : l'hôte donne les touches à l'objet |

**Envoi sans perte.** `⌘↩` envoie `flush`, attend `flushed`, enregistre ce contenu (`update_draft`) puis appelle `send_draft`. Sans cela, les frappes de la dernière demi-seconde ne partiraient pas. Un délai de réponse (1 s) sans `flushed` fait refuser l'envoi, avec un message dans la barre d'état, plutôt que d'envoyer un texte incomplet.

**Le contenu ne revient jamais de l'hôte sans raison.** L'hôte ne renvoie `load` qu'à l'ouverture d'un brouillon, et `replaceSelection` qu'à une action de l'utilisateur. Un rafraîchissement de SwiftUI ne touche pas l'éditeur : son curseur et son historique d'annulation sont préservés.

## 9. Collage

| Touche | Effet |
| --- | --- |
| `⌘V` | **Mode mis en forme** : colle avec la mise en forme. Le HTML du presse-papiers est lu par l'éditeur selon son schéma : ce qui n'y figure pas (tableaux, images, styles, polices hors liste) est retiré, le texte est gardé. **Mode texte seul** : colle le texte |
| `⌥⇧⌘V` | Colle le texte seul, au style de l'endroit du curseur (« Coller et adapter le style » du Mac). L'hôte lit le texte du presse-papiers et l'envoie par `replaceSelection` |

- **Depuis Obsidian** : sa copie en HTML se colle par `⌘V`. À vérifier au prototype : Obsidian doit déposer du HTML (type `public.html`) dans le presse-papiers, et non le code HTML sous forme de texte.
- Les cas de collage réels (Obsidian, Safari, Pages, Gmail, Mail, Notes) sont gardés comme fichiers de test (section 12), avec le résultat attendu.
- **Word** : plus tard. Son HTML (balises et styles `mso-`) passe déjà par le même schéma ; une règle de nettoyage propre à Word pourra être ajoutée.

## 10. Envoi

### Ce que fait le noyau (`nm-store`, `mail_html.rs`)

1. **Liste fermée.** Le HTML du brouillon est réduit, par ammonia, à : `p`, `br`, `h1`, `h2`, `h3`, `ul`, `ol`, `li`, `blockquote`, `strong`, `em`, `u`, `a` (`href` en `http`, `https`, `mailto`), `span` (`style` limité à `color`, `font-family`, `font-size`, aux valeurs des listes de la section 5, et à une police au nom sûr). Toute autre balise est retirée avec son contenu s'il s'agit de `script` ou `style`, sinon remplacée par son texte. Les tailles et les couleurs sont définies **une fois**, dans le noyau (`nm_core::formats`), et transmises à l'éditeur (`compose_formats`, puis `load`) : les deux côtés ne peuvent pas diverger. Les polices viennent du Mac ; le noyau en vérifie le nom.
2. **Styles dans les balises.** Chaque balise reçoit un attribut `style`, pris d'une table fixe (marges des paragraphes, tailles des titres, bordure grise des citations, retrait des listes) : Gmail et Outlook ignorent souvent la balise `<style>`. Aucune couleur de texte ni de fond n'est imposée hors du choix de l'utilisateur.
3. **Liens** : `rel="noopener noreferrer"`.
4. **Document minimal** : `<!DOCTYPE html>`, `<meta charset="utf-8">`, `<body>`, aucune ressource distante.

### Texte brut tiré du HTML (`mail_html::to_text`)

Utilisé pour la partie `text/plain` et pour le passage au texte seul (section 3) : un paragraphe par ligne, comme dans l'éditeur, un paragraphe vide faisant une ligne vide ; un titre sur sa ligne ; une puce `- ` ou un numéro `1. ` par élément, indentés de deux espaces par niveau ; une citation préfixée par `> ` (`>> ` imbriquée) ; un lien dont le texte diffère de l'adresse écrit « texte <adresse> ». Ce n'est pas la conversion du lecteur (`html_text.rs`, citation d'un mail HTML, `reponse.md`, section 4) : réglée pour tout HTML reçu, elle ne numérote ni n'imbrique les listes et ne préfixe pas les citations ; la modifier changerait l'affichage et l'indexation de tous les mails reçus. Celle-ci ne lit que la liste fermée.

### MIME

| `body_format` | Mise en forme réelle | Corps du mail |
| --- | --- | --- |
| `text` | | `text/plain` seul, comme aujourd'hui |
| `html` | Non | `text/plain` seul (texte tiré du HTML) : passer en mode mis en forme sans rien mettre en forme ne change pas le mail |
| `html` | Oui | `multipart/alternative` : `text/plain` puis `text/html` (`charset=utf-8`, `quoted-printable`) |

« Mise en forme réelle » : au moins un titre, une liste, un gras, un italique, un souligné, un lien dont le texte diffère de l'adresse, une couleur, une police ou une taille. Les paragraphes, retours à la ligne, citations et liens automatiques n'en sont pas.

Avec des pièces jointes, `multipart/mixed` dont la première partie est la précédente (`compose::build_message`). La partie HTML compte dans la taille (avertissement à 25 Mo, extension `SIZE`). Les deux versions du MIME (avec et sans `Bcc`, `reponse.md`, section 7) ont les mêmes parties.

## 11. Brouillons, réponses, contrat

- **Réponse** : le texte du mail d'origine est cité, comme aujourd'hui (`reponse.md`, section 4), jamais son HTML. En texte seul, il est préfixé par `> `. Créée directement en mode mis en forme (réglage `compose.format = html`), la citation devient un bloc de citation par la conversion de la section 3.
- **Transfert** : le texte du mail transféré, comme aujourd'hui.
- **Copie dans Drafts** (`append_draft`) : construite comme le mail final. Rouvrir ce brouillon (autre Mac) : une partie HTML le rouvre en mode mis en forme, chargée dans l'éditeur ; sinon en texte seul.
- **Brouillon écrit sur le téléphone** : ils restent en lecture seule (`reponse.md`, section 5). L'éditeur saurait charger leur HTML : les rendre modifiables est une évolution de `brouillons.md`, hors de ce document.
- **Contrat** : `update_draft` accepte `body_format` (`text` ou `html`) ; `prepare_draft`, `create_draft`, `open_draft` le renvoient. `send_message` accepte `body_format`. Le noyau refuse un `html` de plus de 2 Mo (une limite large, contre un contenu aberrant).
- **CLI** et **agent MCP** : ils n'ont pas encore de commande d'envoi (`send_message`, `brouillons.md`, section 8) ; elle prendra `body_format`. `update_draft` et `send_draft` l'acceptent déjà par le socket.
- `compose_formats` (les listes, et `compose.format`) et `html_to_text` (le texte d'un HTML, et s'il a une mise en forme) sont deux méthodes de lecture du contrat.
- **Migration** : aucune. `body_format` accepte déjà `text` et `html` (`schema.sql`).

## 12. Tests et documentation

- **Éditeur** (`apps/editor`, Vitest, `make editor-test`, inclus dans `make test`) : schéma et valeurs hors liste ; chaque raccourci de frappe et clavier ; listes (`↩` sur un élément vide, `⇥`, `⇧⇥`, et `leave` hors liste) ; collages réels conservés en fichiers de test ; idempotence (charger un HTML produit par l'éditeur redonne le même HTML) ; conversion du texte seul vers le document (lignes `>`) ; chaque message du protocole.
- **Noyau** : liste fermée (balises, attributs, schémas de lien, valeurs de style hors liste, `script`, `style`, `on…`) ; styles dans les balises ; `html_to_text` pour chaque élément ; détection de la mise en forme réelle ; les trois cas MIME, avec et sans pièces jointes, avec et sans Cci ; textes non ASCII et emojis.
- **App, tests unitaires** : le routage des touches (`GeekController` contre `FakeCore`) : les commandes de la section 7 sont prises, toutes les autres laissées à l'éditeur, `⎋` laissé quand une boîte est ouverte ; le pont avec une vraie `EditorWebView` hors écran : `load`, `changed`, `flush` avant l'envoi et son délai, `replaceSelection` en une étape d'annulation, aucun `load` sur un rafraîchissement.
- **App, scénarios d'interface** : la fenêtre du mode scénario se dit fenêtre clé (`ScriptWindow`) sans que l'app soit activée, WebKit ne donnant les touches à sa page que dans la fenêtre clé. `⌘V` lit le presse-papiers du Mac, qu'un scénario ne doit jamais toucher : les collages sont vérifiés par les tests de l'éditeur. Écrire un texte accentué, mettre en forme par `⌘B` et un raccourci de frappe, coller un HTML de test par `⌘V` puis `⌥⇧⌘V`, basculer par `⇧⌘T`, envoyer juste après une frappe (le texte entier part), `⌘⎋` et `⌃I`. Instantanés en thème clair et sombre. `Runner.state()` expose le mode, la barre, et le contenu de l'éditeur (par `flush`). Les scénarios d'écriture existants (`writing`, `textinput`, `compose`, `recipients`, `ai`, `drafts`) sont repris.
- **Documentation** : `docs/sending.md` (modes, mise en forme, collage, MIME), `docs/geek-ui.md` (touches du contexte « edition »), un nouveau `docs/editor.md` (composant, protocole, construction), aide de l'app (`Help.swift`), `README.md` (Node pour construire).

## 13. Prototype et ordre de livraison

**Étape 0 : prototype**, dans une branche à part, jugé sur des critères éliminatoires dans l'app elle-même (pas dans un navigateur) :

1. Le correcteur orthographique souligne et corrige, en français et en anglais.
2. Writing Tools corrige et réécrit un paragraphe sans casser le document.
3. Touches mortes (accents sur clavier AZERTY), dictée, palette d'emojis. Les accents par appui prolongé ne sont pas exigés.
4. `⌥←` `⌥→`, `⇧⌥↓`, `⌘←`, `⌃A` `⌃E` `⌃K` se comportent comme dans Mail.
5. `⌘Z` et le menu Édition atteignent l'historique de l'éditeur.
6. `⌘↩` juste après une frappe envoie le texte complet.
7. **Un scénario d'interface tape dans l'éditeur**, app inactive et hors écran. Sinon, une autre stratégie de test est à définir avant d'aller plus loin.
8. Ouverture d'une réponse avec la vue préchargée : moins de 100 ms ; mémoire du processus WebKit mesurée.
9. Copie HTML d'Obsidian, collage depuis Safari et Pages : réduits au schéma.

**Étape 1** : l'éditeur en mode texte seul remplace `TextBuffer` pour le texte du mail (le code et ses tests sont retirés), avec le nouveau routage des touches (`⌃I`, `⌘⎋`). Le mail envoyé ne change pas.
**Étape 2** : le mode mis en forme, la barre, `⇧⌘T`, l'envoi HTML par le noyau.
**Étape 3** : `⌥⇧⌘V`, les cas de collage de test, le CLI et l'agent MCP.

## 14. Questions ouvertes

- **`⌃I`** : vérifier qu'aucune commande de texte de WebKit ne l'utilise (en ASCII, `⌃I` est la tabulation) ; l'app la prend de toute façon avant l'éditeur.
- **IA et mise en forme** : réalisé, l'IA ne travaille que sur un brouillon en texte seul ; l'app le dit sur un brouillon mis en forme (« L'IA ne travaille que sur du texte pour l'instant · ⇧⌘T pour le texte seul ») et le noyau refuse `ai_run`. Ses textes appliqués vont à l'éditeur comme une seule étape d'annulation (`replaceRange`). Lui donner le HTML, pour qu'elle garde gras et listes, est une évolution de `ia.md`.
- **Les deux teintes de couleur restantes**, et les valeurs exactes de la palette : avec le design, vérifiées en contraste sur fond blanc et sur fond sombre.
- **Signature** : quand elle existera, elle utilise ce schéma.
- **Images dans le texte** : exclues en v1 ; le schéma de la base les prévoit (`content_id`, `is_inline`).
