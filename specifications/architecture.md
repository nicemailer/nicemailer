# NiceMailer – Document d'architecture

*Version du 2026-10-16*

NiceMailer est un client mail pour macOS : un front SwiftUI, un noyau Rust réutilisable par quatre autres interfaces (CLI, TUI, web, MCP), et une installation qui reste un simple double-clic pour le grand public.

---

## 1. Contexte et objectifs

L'application est découpée en deux parties : le front en Swift pour l'interface Mac, le back en Rust pour toute la logique mail. Le noyau Rust ne doit pas être pensé comme la bibliothèque d'une app, mais comme un produit à part entière dont l'app Mac n'est que le premier client.

### Objectifs

- Une seule logique métier (synchro, threading, recherche, envoi), partagée par toutes les interfaces.
- Une architecture compatible avec cinq interfaces : app macOS, CLI, TUI, web local et serveur MCP.
- Une interface fluide, même sur de grosses boîtes aux lettres.
- Une expérience grand public : téléchargement d'un DMG, double-clic, aucun terminal requis.
- Une distribution directe (GitHub Releases), sans passer par le Mac App Store.

### Contraintes structurantes

- La fluidité de l'interface est un critère de premier rang : toute lecture est incrémentale et paginée.
- Un utilisateur non technique ne doit jamais avoir à connaître l'existence du noyau, du CLI ou du protocole.
- Les interfaces secondaires (CLI, MCP, web) doivent fonctionner sans conflit avec l'app, y compris quand elle est ouverte.

---

## 2. Principes directeurs

Le noyau ne connaît aucune interface : il expose un contrat unique, et chaque front n'est qu'un adaptateur fin autour de ce contrat.

1. **Noyau agnostique.** `nm-core` et `nm-engine` ne dépendent jamais d'une crate d'interface. Les dépendances vont toujours des adaptateurs vers le noyau.
2. **Un seul contrat.** Commandes, requêtes et événements forment la seule surface partagée. Swift l'utilise comme le CLI, le TUI, le web et le MCP.
3. **Aucune logique métier côté Swift.** Si le front décide du threading, des règles de tri ou des brouillons, cette logique sera dupliquée dans les autres interfaces. Swift fait de l'affichage et de l'interaction. Exception délimitée : l'édition du texte d'un mail est confiée à un composant d'édition web (Tiptap), commun à toutes les interfaces graphiques ; il porte la logique d'édition (schéma, collage), et le noyau garde la règle de ce qui est envoyé (`edition.md`, section 2).
4. **Un seul propriétaire de l'état.** Un unique processus noyau possède la base SQLite et la synchro ; tous les clients lui parlent.
5. **Vues prêtes à afficher.** Le contrat expose des structures de présentation (`ThreadSummary`, `MessageView`) plutôt que des lignes brutes de la base.
6. **Pagination partout.** Jamais de « charge tout » : une fenêtre et un curseur, pour tenir la promesse de fluidité.
7. **Complexité cachée.** Tout ce qui est technique (socket, CLI, MCP) s'active depuis l'interface, sans terminal.
8. **Le noyau n'interagit jamais avec l'utilisateur.** Il n'affiche rien et ne pose aucune question : notifications macOS, fenêtres et dialogues sont l'affaire des clients, et en pratique de l'app Swift.

---

## 3. Vue d'ensemble

Un seul processus noyau héberge le moteur ; l'app et toutes les autres interfaces sont des clients du même socket Unix. Le noyau démarre à la première connexion d'un client, ou dès l'ouverture de session si l'option de tri en arrière-plan est activée, ce qui est le cas par défaut (section 6).

```
 ┌───────────────┐ ┌─────────┐ ┌─────────┐ ┌──────────────┐ ┌─────────────┐
 │ App SwiftUI   │ │   CLI   │ │   TUI   │ │ nm-web       │ │  nm-mcp     │
 │ (client)      │ │         │ │         │ │ HTTP/WS      │ │ stdio→socket│
 └───────┬───────┘ └────┬────┘ └────┬────┘ └──────┬───────┘ └──────┬──────┘
         │              │           │             │                │
         └──────────────┴─────┬─────┴─────────────┴────────────────┘
                              │  JSON-RPC 2.0 sur socket Unix
                              ▼
                  ┌───────────────────────┐
                  │  nm-daemon (noyau)    │  ← LaunchAgent, toujours actif
                  │  ┌─────────────────┐  │
                  │  │   nm-engine     │  │
                  │  └──┬─────┬─────┬──┘  │
                  │  nm-proto │  nm-store │
                  │  (IMAP…)  │  (SQLite) │
                  └───────────┴───────────┘
                         │
                  serveurs mail (IMAP / SMTP / JMAP)
```

---

## 4. Organisation du workspace Cargo

Le dépôt est un workspace Cargo de dix crates plus l'app SwiftUI ; chaque crate a une responsabilité unique et les interfaces restent des binaires minces.

| Crate       | Rôle                                                                     | Dépend de                         |
| ----------- | ------------------------------------------------------------------------ | --------------------------------- |
| `nm-core`   | Domaine pur : modèles, règles, état, sans I/O direct                     | rien d'interne                    |
| `nm-proto`  | Adaptateurs réseau : IMAP d'abord, SMTP, OAuth2, puis JMAP               | `nm-core`                         |
| `nm-store`  | Stockage local : SQLite, corps des messages, index plein texte              | `nm-core`                         |
| `nm-engine` | Orchestration : synchro, file d'envoi, recherche, émission d'événements  | `nm-core`, `nm-proto`, `nm-store` |
| `nm-api`    | Contrat public stable : commandes, requêtes, événements, versionnage     | `nm-core`                         |
| `nm-daemon` | Processus noyau : serveur JSON-RPC sur socket Unix, hébergeant le moteur | `nm-engine`, `nm-api`             |
| `nm-cli`    | Binaire en ligne de commande, client du socket                           | `nm-api`                          |
| `nm-tui`    | Interface terminal (ratatui), client du socket                           | `nm-api`                          |
| `nm-web` | Passerelle HTTP/WebSocket locale pour l'interface web                    | `nm-api`                          |
| `nm-mcp`    | Serveur MCP, proxy stdio vers le socket                                  | `nm-api`                          |

```
nicemailer/
├── crates/
│   ├── nm-core/  nm-proto/  nm-store/  nm-engine/  nm-api/
│   └── nm-daemon/  nm-cli/  nm-tui/  nm-web/  nm-mcp/
└── apps/
    └── NiceMailer/        # app macOS SwiftUI
```

L'app Swift dialogue avec le noyau par le même protocole que les autres clients, sans pont FFI. Ce choix remplace l'idée initiale d'un pont UniFFI : tant que tous les clients passent par le socket, il n'y a qu'un seul contrat à maintenir, et le CLI teste exactement ce que voit l'app.

---

## 5. Contrat entre le noyau et les clients

Le contrat tient en trois familles de messages, transportés en JSON-RPC 2.0 sur un socket Unix : c'est la seule chose que tous les fronts partagent.

| Famille    | Sens                              | Exemples                                                               |
| ---------- | --------------------------------- | ---------------------------------------------------------------------- |
| Commandes  | client vers noyau, écriture       | `send_message`, `move_to_folder`, `mark_read`, `add_account`, `delete` |
| Requêtes   | client vers noyau, lecture        | `list_threads(folder, cursor)`, `get_message(id)`, `search(query)`     |
| Événements | noyau vers clients, notifications | `ThreadChanged`, `MessageArrived`, `AccountError`, `ActionReverted`, `ResyncRequired` (catalogue complet dans `evenements.md`) |

### Règles du contrat

- **Pagination par curseurs.** Toute liste retourne une fenêtre et un curseur ; aucune requête ne renvoie une boîte entière.
- **Vues de présentation.** Les réponses sont des structures prêtes à afficher, indépendantes du schéma SQLite.
- **Événements en notifications JSON-RPC.** Les clients s'abonnent après la connexion ; l'app Swift les consomme via un `AsyncStream`. Les événements signalent ce qui a changé sans porter l'état ; ils sont numérotés (époque et séquence) pour qu'un client détecte une perte et se rattrape, par rejeu ou par rechargement complet. Le mécanisme est spécifié dans `evenements.md`.
- **Poignée de main de version.** À la connexion, le client annonce sa version de protocole ; si elle est trop ancienne, le noyau répond par une erreur explicite plutôt que de planter.
- **Requêtes d'une connexion en parallèle** (ajouté en novembre 2026). Sur une même connexion, les lectures s'exécutent chacune de leur côté, sans attendre les autres requêtes ; les commandes s'exécutent l'une après l'autre, dans l'ordre d'envoi ; `hello` et `subscribe` sont traités à la lecture. Les réponses peuvent donc revenir dans un autre ordre que les requêtes : le client les associe par `id`, comme JSON-RPC le prévoit. Un client qui veut qu'une lecture voie une écriture attend la réponse de l'écriture avant d'envoyer la lecture. Raison : l'app n'a qu'une connexion ; pendant l'import d'un compte, ses rechargements de listes passaient avant ce que l'utilisateur attendait (la réponse s'ouvrait en dix secondes).
- **Erreurs typées.** Les codes d'erreur JSON-RPC sont stables et documentés, pour que le CLI et le MCP produisent des messages utiles.
- **Fichiers par chemin.** Le contenu d'une pièce jointe ne transite jamais dans le JSON : le noyau et les clients échangent des chemins de fichiers locaux (voir ci-dessous).

### Pièces jointes : transfert par chemin

Tous les clients tournent sur la même machine et sous le même utilisateur que le noyau. Échanger un chemin évite d'encoder en base64 des fichiers de plusieurs dizaines de mégaoctets dans le JSON, et permet à l'app de les passer directement à Quick Look, au Finder ou à l'app par défaut.

**Lecture d'une pièce jointe.** La requête `get_attachment(attachment_id)` :

1. extrait la pièce jointe du message brut si elle ne l'est pas déjà (`stockage-brut.md`, section 6) ;
2. renvoie `{path, filename, mime_type, size}`, où `path` est un chemin absolu dans `<data_dir>/attachments.noindex/`.

Le fichier appartient au noyau. Les clients le lisent, le copient ou l'ouvrent, mais ne le modifient ni ne le déplacent (droits `0400`). Le chemin est valable tant que le message existe ; les fichiers extraits peuvent toutefois être purgés et réextraits à tout moment. Un client ne conserve donc pas le chemin : il le redemande avant chaque usage.

- **Enregistrer sous / glisser vers le Finder** : le client copie le fichier vers la destination choisie par l'utilisateur.
- **Images inline (`cid:`)** : l'app résout les URL `cid:` du HTML par un gestionnaire de schéma d'URL de `WKWebView`, qui demande le chemin au noyau et lit le fichier.
- **Serveur web et MCP** : `nm-web` lit le fichier et le transmet au navigateur. Pour le MCP, le proxy renvoie le chemin ; s'il faut aussi renvoyer le contenu à l'agent, la décision sera prise en codant `nm-mcp`.

**Sécurité des fichiers extraits.**

- Le nom du fichier vient de l'expéditeur, donc il n'est pas fiable. Il est assaini avant d'écrire sur le disque : pas de séparateur de chemin, pas de `..`, pas de caractère de contrôle, longueur bornée. Le nom d'origine n'est conservé que dans `attachment.filename`, pour l'affichage.
- Le noyau pose l'attribut étendu `com.apple.quarantine` sur chaque fichier extrait, comme le fait Mail. À l'ouverture d'un exécutable ou d'une archive, Gatekeeper avertit l'utilisateur.

**Envoi d'une pièce jointe.** Le client donne le chemin du fichier à joindre (`add_draft_attachment`, ou `send_message` pour un envoi direct). Le noyau le lit **au moment de la commande** et le copie dans sa zone de stockage privée, `outgoing.noindex/` (`brouillons.md`, section 7). Un fichier déplacé ou supprimé ensuite, pendant que le brouillon attend ou avant l'envoi effectif hors ligne, ne pose donc pas de problème. Le noyau n'a pas plus de droits que le client, puisqu'il tourne sous le même utilisateur : lire un chemin fourni par le client ne lui donne accès à rien de nouveau. Pour un envoi demandé par un agent, la confirmation dans l'app affiche la liste des pièces jointes avec leur chemin d'origine (section 9).

### Pourquoi JSON-RPC

Le format est lisible et débogable à la main, le serveur MCP en est presque un simple adaptateur, et l'outillage Swift reste léger. gRPC serait plus rigoureux mais alourdirait la chaîne pour un gain faible ici.

### Exemple d'échange

```json
→ {"jsonrpc":"2.0","id":1,"method":"list_threads","params":{"folder":"inbox","limit":50,"cursor":null}}
← {"jsonrpc":"2.0","id":1,"result":{"items":[ /* ThreadSummary… */ ],"next_cursor":"c_8f2a","as_of":{"epoch":"e_7c1","seq":4218}}}
← {"jsonrpc":"2.0","method":"MessageArrived","params":{"epoch":"e_7c1","seq":4219,"message_id":"m_5521","folder_id":"f_1"}}
```

---

## 6. Processus et packaging macOS

Le noyau est un LaunchAgent embarqué dans le bundle, enregistré par l'app via `SMAppService.agent(plistName:)`. Il démarre à la demande, dès qu'un client (app, CLI, TUI, MCP) en a besoin. Une seule option, « Continuer à trier mes mails quand NiceMailer est fermé », activée par défaut, le fait tourner en permanence depuis l'ouverture de session. L'app, le CLI, la TUI et le MCP en sont de simples clients, utilisables en même temps.

### Options étudiées

| Option                        | Principe                                                                    | Verdict                                                                   |
| ----------------------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| A. Moteur dans l'app          | Le noyau tourne dans l'app, socket ouvert tant qu'elle est lancée           | Simple, mais CLI/MCP/web inutilisables app fermée, pas de synchro en fond |
| **B. Agent de fond embarqué** | Noyau dans le bundle, enregistré comme agent ; tous les fronts sont clients | **Retenue**                                                               |
| C. Démon installé à part      | Service séparé (Homebrew, pkg)                                              | Exclut le grand public                                                    |
| D. Hybride FFI + socket       | App liée au noyau via UniFFI, socket pour les autres                        | Deux chemins, comportements qui divergent                                 |

Réalisé en développement (novembre 2026, `docs/app-bundle.md`) : le bundle avec le noyau et le CLI, l'agent enregistré par `SMAppService`, le démarrage à la demande par l'app, l'arrêt (`nicemailer core stop`, code 0), et le choix de la base dans l'app (⌥ à l'ouverture, « Changer de base… »), mémorisé dans le fichier `data-dir` du dossier par défaut, que le noyau et le CLI lisent aussi. Avant de changer de base, l'app fait vérifier le dossier par le CLI (`nicemailer mailbox check`) : une base que le noyau refuserait (version de développement ancienne, version plus récente) est refusée et la base actuelle conservée ; une base d'un format plus ancien n'est mise à jour (`nicemailer mailbox upgrade`) qu'après accord de l'utilisateur. Un noyau qui s'arrête sur une erreur en écrit la raison dans `core-error.json` de sa base, que l'app affiche à la place de « noyau injoignable ». L'identifiant de l'app reste `eu.nicemailer.NiceMailer`. Mises à jour par Sparkle et mode maintenance du noyau réalisés (`mise-a-jour.md`, `docs/updates.md`). Restent l'option de tri en arrière-plan, le déplacement dans `/Applications`, les liens du CLI, la désinstallation, la TUI et le MCP.

### Structure du bundle

```
NiceMailer.app/Contents/
├── MacOS/NiceMailer                                  ← app SwiftUI (client)
├── Library/LaunchAgents/eu.nicemailer.core.plist     ← déclare le noyau
└── Helpers/
    ├── nicemailer-core                               ← noyau Rust (nm-daemon)
    ├── nicemailer                                    ← CLI
    ├── nicemailer-tui                                ← TUI
    ├── nicemailer-mcp                                ← serveur MCP (proxy stdio)
    └── nicemailer-web                                ← passerelle web (nm-web), second temps
```

`nicemailer-web` fait partie de la structure cible, mais ne sera développé que dans un second temps (section 12).

### Contraintes de launchd

Vérifiées sur macOS 27 (page de manuel `launchd.plist` et agent de test) :

- **`~` n'est pas développé dans `SockPathName`** : le chemin reste littéral. Un plist embarqué dans le bundle ne connaît pas le répertoire personnel de l'utilisateur et ne peut donc pas déclarer un socket dans `~/Library`.
- **`SecureSocketWithKey`** publie le chemin du socket dans une variable d'environnement, comme `ssh-agent` avec `SSH_AUTH_SOCK`. Elle n'est transmise qu'aux processus lancés après le chargement de l'agent : un terminal déjà ouvert ou une session SSH ne l'ont pas. Trop fragile pour le CLI.
- **`KeepAlive` implique `RunAtLoad`**, y compris sous la forme `SuccessfulExit` : on ne peut pas avoir « relancé après un plantage » sans « démarré à l'ouverture de session ».
- **Le plist d'un agent `SMAppService` est figé** dans le bundle signé : `RunAtLoad` ne peut pas dépendre d'une option à l'exécution.

En conséquence, l'activation du noyau par son socket (clé `Sockets`) est écartée : le noyau crée lui-même son socket, et c'est lui qui applique l'option de tri en arrière-plan.

### Déclaration de l'agent

Un seul agent, `eu.nicemailer.core`, enregistré par l'app à chaque lancement s'il ne l'est pas déjà :

```xml
<key>Label</key>                       <string>eu.nicemailer.core</string>
<key>BundleProgram</key>               <string>Contents/Helpers/nicemailer-core</string>
<key>RunAtLoad</key>                   <true/>
<key>KeepAlive</key>                   <dict><key>SuccessfulExit</key><false/></dict>
<key>AssociatedBundleIdentifiers</key> <string>eu.nicemailer.NiceMailer</string>
```

### Cycle de vie du noyau

- **Démarrage.** launchd lance le noyau à l'ouverture de session (`RunAtLoad`) et chaque fois qu'un client le demande (voir ci-dessous). Le noyau prend le verrou d'instance unique (`lock`, voir `stockage-brut.md`), supprime un socket laissé par un plantage, puis écoute sur son socket.
- **Option de tri en arrière-plan activée** (défaut) : le noyau reste actif en permanence. Un plantage (code de sortie non nul) est relancé par launchd.
- **Option désactivée** : le noyau s'arrête de lui-même, avec le code 0, après `core.idle_exit_secs` sans client connecté (60 s par défaut), y compris juste après son démarrage. À l'ouverture de session, il s'arrête donc au bout de ce délai. Une sortie avec le code 0 n'est pas relancée par launchd.
- **Arrêt propre.** Sur SIGTERM, le noyau termine les transactions en cours et ferme les connexions IMAP avant de sortir.

L'option est la clé `core.background_sorting` de `config.toml` (`configuration.md`), modifiable depuis l'app, le CLI ou à la main ; elle prend effet sans redémarrage.

### Démarrage à la demande par les clients

Tous les clients (app, CLI, TUI, MCP) suivent la même procédure :

1. se connecter au socket ;
2. en cas d'échec (socket absent ou personne à l'écoute), exécuter `launchctl kickstart gui/<uid>/eu.nicemailer.core` ;
3. réessayer avec un délai croissant, pendant 3 s au plus ;
4. si le noyau répond par l'erreur `Maintenance` (mise à jour ou migration en cours), attendre sans lancer de `kickstart` (`mise-a-jour.md`) ;
5. si l'agent n'est pas enregistré ou a été désactivé par l'utilisateur, afficher une erreur explicite avec la marche à suivre (voir « Cas limites »).

Deux clients qui démarrent en même temps ne posent pas de problème : launchd ne lance qu'une instance par label, et le verrou d'instance couvre le reste.

### Points techniques

- **LaunchAgent plutôt que LoginItem.** Un exécutable Rust nu s'enregistre mal comme LoginItem, qui attend un mini-bundle d'app.
- **Emplacement du socket.** `~/Library/Application Support/NiceMailer/core.sock`, créé par le noyau, droits `0600` (environ 60 caractères, sous la limite de 104 des sockets Unix). Les clients sont authentifiés par leur signature (section 9).
- **Emplacement de l'app.** Lancée depuis le DMG ou le dossier Téléchargements, l'app est exécutée depuis un chemin temporaire aléatoire (App Translocation) et l'agent pointerait vers ce chemin. Au premier lancement, l'app propose donc de se déplacer dans `/Applications` avant d'enregistrer l'agent.
- **Versions alignées.** App, noyau et helpers sont livrés ensemble dans un seul bundle, ce qui évite les incompatibilités ; la poignée de main de version couvre les cas résiduels (ancien CLI installé, par exemple).
- **Mise à jour.** Pendant une mise à jour Sparkle et la migration éventuelle de la base, le noyau est en mode maintenance et ne sert aucun client. Le déroulement est spécifié dans `mise-a-jour.md`.
- **Notifications.** Seule l'app peut afficher des notifications macOS, à partir des événements du noyau (`MessageArrived`). App fermée, il n'y a donc pas de notification de nouveau mail : c'est voulu, le noyau n'interagit pas avec l'utilisateur (principe 8).
- **Secrets et Keychain.** Toute la logique des secrets est dans le noyau. Il crée, lit, met à jour et supprime lui-même les entrées du Keychain ; les clients ne les touchent jamais. Pour ajouter un compte, le client transmet le mot de passe au noyau dans la commande `add_account`, sur le socket authentifié (section 9). Pour OAuth2, le noyau prépare la requête d'autorisation et écoute la redirection sur `127.0.0.1` ; il renvoie au client l'URL d'autorisation, et c'est le client qui ouvre le navigateur. Le flux est spécifié dans `oauth2.md`. Le noyau est un exécutable sans bundle ni profil de provisionnement : il utilise le trousseau de session classique (« login »), et non le trousseau à protection de données, qui exige des droits liés à un profil. À valider dans le squelette (section 11).
- **Pilotage avancé.** `nicemailer core status | start | stop | restart` donne la main sur le noyau. `nicemailer setup` lance l'app en arrière-plan (`open -g`) pour enregistrer l'agent s'il ne l'est pas, par exemple après une installation par Homebrew sans ouverture de l'app.

### Cas limites

- **NiceMailer désactivé dans Réglages Système > Général > Ouverture.** L'agent est alors entièrement désactivé : macOS n'a pas de position « autorisé, mais pas à l'ouverture de session », et le démarrage à la demande échoue aussi. L'app affiche un bandeau avec un bouton qui ouvre ce panneau (`SMAppService.openSystemSettingsLoginItems()`) ; le CLI indique le chemin du réglage. C'est pourquoi l'option de tri en arrière-plan est dans l'app : elle permet d'éviter le démarrage à l'ouverture de session sans tout désactiver.
- **App déplacée.** À chaque lancement, l'app vérifie l'état de l'agent et le réenregistre si besoin. Elle répare aussi les liens du CLI et l'entrée MCP de Claude Desktop, qui pointent vers des binaires du bundle.
- **Désinstallation.** Le menu « Désinstaller NiceMailer… » retire l'agent, les liens du CLI et l'entrée MCP, et propose de supprimer les données. Si l'app est simplement mise à la corbeille, l'agent reste enregistré mais ne peut plus rien lancer, et les liens du CLI sont cassés : c'est inoffensif, et documenté dans l'aide.

---

## 7. Expérience utilisateur non technique

L'utilisateur n'installe qu'une chose, l'app ; tout le reste s'active depuis les Réglages, sans terminal.

| Fonction | Ce que voit l'utilisateur | Ce qui se passe |
| --- | --- | --- |
| Premier lancement hors de `/Applications` | Proposition de déplacer l'app dans `/Applications` | Évite l'App Translocation, qui fausserait l'enregistrement de l'agent |
| Premier lancement | La notification macOS « Élément d'arrière-plan ajouté », puis un écran de bienvenue avec l'option « Continuer à trier mes mails quand NiceMailer est fermé », cochée | L'app enregistre l'agent via `SMAppService` |
| Tri en arrière-plan | Rien à faire : les mails sont reçus et triés app fermée, y compris après un redémarrage | Le noyau démarre à l'ouverture de session et reste actif. Option désactivée : il ne tourne que tant qu'un client est connecté |
| NiceMailer désactivé dans les réglages système | Un bandeau dans l'app avec un bouton vers le bon panneau | Le démarrage à la demande est impossible tant que l'agent est désactivé |
| CLI et TUI | Bouton « Installer les commandes `nicemailer` et `nicemailer-tui` » | Liens dans `/usr/local/bin` après demande du mot de passe administrateur ; le cask Homebrew les crée aussi |
| Désinstallation | Menu « Désinstaller NiceMailer… » | Retrait de l'agent, des liens du CLI et de l'entrée MCP ; suppression des données sur demande |
| MCP | Bouton « Connecter à Claude » | Écriture de l'entrée dans la config de Claude Desktop, ou bloc JSON à copier |
| Web | Bouton « Ouvrir dans le navigateur » | Serveur sur `127.0.0.1` avec jeton aléatoire, désactivé par défaut |
| Mises à jour | Fenêtre Sparkle, puis un bandeau « Mise à jour de NiceMailer… » pendant l'installation et la migration | Remplacement de tout le bundle, noyau compris ; noyau en mode maintenance (`mise-a-jour.md`) |

---

## 8. Stockage et concurrence

Le noyau est l'unique propriétaire de la base SQLite ; aucun client n'y accède directement.

- **SQLite en mode WAL** : lectures concurrentes fluides pendant les écritures de synchro.
- **Corps de messages** : table séparée, lue à la demande pour garder les listes légères. C'est une interprétation du message brut, réécrite quand l'analyseur évolue ; seul le brut sur disque est immuable.
- **Recherche plein texte** : index FTS dans `nm-store`, interrogé via la requête `search`.
- **Runtime async unique** (tokio) côté noyau : les clients ne gèrent aucun thread réseau.
- **Pas d'accès partagé à la base** : l'app Swift, le CLI et le MCP passent tous par le noyau, ce qui supprime les conflits de verrouillage.
- **Traçabilité** : les entités portent leurs dates et sources de création et de modification, et un journal en ajout seul garde l'historique des décisions et des actions (`tracabilite.md`).
- **Plusieurs Mac** : chaque Mac a sa propre base. Les décisions de screening et la configuration d'une boîte se partagent entre Mac par le serveur IMAP de cette boîte, sans service tiers (`synchronisation-configuration.md`).

---

## 9. Sécurité du socket et des clients

Le noyau vérifie la signature de code de chaque client qui se connecte au socket et lui accorde des droits selon cette signature ; l'envoi d'un mail par un agent exige toujours une confirmation de l'utilisateur.

### Menaces couvertes et non couvertes

Un processus de l'utilisateur qui peut se connecter au socket peut aussi lire directement la base et les fichiers bruts (TCC ne protège pas les données d'une app hors bac à sable), ou exécuter le CLI signé de NiceMailer. Sans bac à sable, macOS n'offre pas de frontière solide entre deux processus du même utilisateur. Authentifier le socket ne protège donc pas la lecture des mails contre un code malveillant qui tourne sous le compte de l'utilisateur.

| Menace                                               | Protection                                                                                       |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Autres comptes de la machine                         | Socket en `0600`, dossier de données (base, brut, socket) en `0700`                              |
| Apps en bac à sable (App Store)                      | Déjà bloquées : elles n'ont pas accès au chemin du socket                                        |
| Processus quelconque qui parle directement au socket | Vérification de la signature du client                                                           |
| Agent IA qui abuse d'un client légitime              | Droits par client, confirmation des envois, journal, limite de débit                             |
| Code malveillant du même utilisateur, déterminé      | **Non couvert.** Seul un chiffrement au repos le limiterait (décision reportée, voir section 11) |

### Vérification de la signature

À chaque connexion, le noyau :

1. récupère le jeton d'audit du client par `getsockopt(LOCAL_PEERTOKEN)`. Le jeton d'audit est préféré au PID (`LOCAL_PEERPID`), qui peut être réattribué entre la connexion et la vérification ;
2. obtient la signature du processus par `SecCodeCopyGuestWithAttributes` (attribut `kSecGuestAttributeAudit`) et la valide par `SecCodeCheckValidity` contre l'exigence :
   ```
   anchor apple generic and certificate leaf[subject.OU] = "<TEAM_ID>"
     and identifier in ("eu.nicemailer.NiceMailer", "eu.nicemailer.cli", "eu.nicemailer.tui",
                        "eu.nicemailer.mcp", "eu.nicemailer.web")
   ```
3. ferme la connexion si la vérification échoue, et l'inscrit au journal.

**L'identité du client est celle de sa signature**, jamais ce qu'il annonce dans la poignée de main. Chaque binaire a un identifiant de signature distinct, qui détermine ses droits.

Tous les binaires sont signés avec le *hardened runtime* (qui bloque l'injection par `DYLD_*`) et la validation des bibliothèques, pour qu'un binaire signé ne charge pas de code étranger.

En développement, les builds signés ad hoc ne satisfont pas l'exigence. Une exigence assouplie est activée **à la compilation** (`cfg(debug_assertions)`), jamais par une variable d'environnement, qui deviendrait une porte dérobée en production.

### Droits par client

| Client      | Lecture | Modification (flags, déplacements, suppression)            | Envoi                              | Comptes et réglages |
| ----------- | ------- | ---------------------------------------------------------- | ---------------------------------- | ------------------- |
| App         | oui     | oui                                                        | oui                                | oui                 |
| CLI, TUI    | oui     | oui                                                        | oui                                | oui                 |
| Serveur web | oui     | oui                                                        | oui                                | non                 |
| MCP (agent) | oui     | désactivé par défaut, activable dans l'app seulement (réglage protégé) ; groupes de contacts permis, sauf leur suppression (`mcp.md`, section 4) | **toujours soumis à confirmation** | non                 |

Les réglages qui étendent les droits d'un client (modification par l'agent MCP, activation du serveur web) sont des **réglages protégés** : ils ne sont pas dans `config.toml` ni `accounts.toml`, que n'importe quel programme peut modifier, mais en base, et seule l'app peut les changer (`configuration.md`, section 7). Les autres réglages sont dans `config.toml`, les comptes dans `accounts.toml` ; la colonne « Comptes et réglages » désigne leur modification par le contrat.

L'identité vérifiée du client sert aussi à la traçabilité : le noyau inscrit lui-même la source de chaque création et modification (`created_by`, `modified_by`, `audit_log`), sans jamais la demander au client. Seuls l'app, la TUI et le serveur web peuvent marquer un message comme consulté (`mark_viewed`). Voir `tracabilite.md`.

Le serveur web est un client signé du socket. Il authentifie le navigateur par un jeton aléatoire, propre à la session, et il est le seul client à utiliser un jeton. Pour le socket lui-même, un jeton ne servirait à rien : il serait stocké dans un fichier `0600` que le code malveillant visé peut lire aussi.

### Sécurité du serveur web : à traiter

La sécurité du serveur web reste à spécifier. Les points connus sont :

- le transport du jeton : passé dans l'URL, il finit dans l'historique du navigateur ;
- la protection contre le DNS rebinding (vérification de l'en-tête `Host`) ;
- la vérification de l'`Origin` des connexions WebSocket ;
- les droits accordés au navigateur.

`nm-web` n'est pas prioritaire. Les décisions d'implémentation seront prises quand on commencera à coder ce module, pas avant. D'ici là, les droits du tableau ci-dessus et le jeton aléatoire ne sont que des orientations.

### Confirmation des envois demandés par un agent

Un mail entrant est un contenu hostile, lu par un LLM qui dispose d'outils. Sans confirmation, une instruction cachée dans un mail suffirait à exfiltrer des données par un envoi. Le noyau applique donc la règle suivante à toute commande `send_message` venant du client MCP :

1. Le message est mis en attente de confirmation. Il n'entre pas dans la file d'envoi.
2. Le noyau émet l'événement `SendConfirmationRequested`. Le noyau ne lance pas l'app (principe 8) : c'est le proxy MCP qui, recevant la demande de confirmation, ouvre l'app si elle est fermée (`open -g -b eu.nicemailer.NiceMailer`). À son ouverture, l'app récupère les demandes en attente par `list_pending_confirmations`.
3. L'app affiche le message complet (destinataires, objet, corps, pièces jointes) avec « Envoyer » et « Refuser ».
4. Seule l'app peut confirmer : la commande `confirm_send` est refusée à tout autre client, MCP compris.
5. Sans réponse après un délai (paramètre à fixer), la demande expire et vaut refus.
6. L'agent reçoit le résultat (`sent`, `refused` ou `expired`) en réponse à sa commande.

**Limite assumée** : un agent qui dispose aussi d'un accès au terminal peut appeler `nicemailer send` et contourner cette confirmation, puisque le CLI a le droit d'envoyer. La confirmation protège le canal MCP, pas l'utilisateur contre un agent auquel il a confié son terminal.

### Journal et limites

- Les actions de tous les clients, app comprise, sont journalisées dans `audit_log` et consultables dans l'app : source, action, entité concernée, date (`tracabilite.md`).
- Les suppressions et les demandes d'envoi des clients autres que l'app sont soumises à une limite de débit.

---

## 10. Distribution directe via GitHub

La distribution hors App Store lève les contraintes de sandbox : socket local, lien symbolique du CLI et écriture dans la config de Claude Desktop restent possibles.

### Signature et notarisation

- Compte Apple Developer (99 €/an) et certificat **Developer ID Application**.
- Signature de chaque binaire Rust **avant** celle du bundle, de l'intérieur vers l'extérieur, avec le *hardened runtime*.
- Notarisation par `notarytool`, puis `stapler` pour agrafer le ticket au DMG.
- Binaires universels (`aarch64` + `x86_64` fusionnés avec `lipo`), sauf abandon d'Intel.

### Chaîne de publication (GitHub Actions, runner macOS, tag `vX.Y.Z`)

1. Build Rust (deux cibles, fusion `lipo`) et build Swift.
2. Assemblage du bundle, signature, notarisation.
3. Création du DMG et publication dans les GitHub Releases.
4. Génération de l'`appcast.xml` Sparkle (signature EdDSA), hébergé sur GitHub Pages ou en asset de release.

Secrets requis : certificat `.p12` en base64, clé API App Store Connect pour `notarytool`, clé privée Sparkle.

### Canal complémentaire

Un cask Homebrew (`brew install --cask nicemailer`) pointant vers la même release, pour les utilisateurs techniques.

---

## 11. Risques et décisions ouvertes

| Sujet                   | Risque ou question                            | Piste                                                         |
| ----------------------- | --------------------------------------------- | ------------------------------------------------------------- |
| Protocoles              | IMAP seul au départ, ou JMAP/Exchange aussi ? | Commencer par IMAP ; garder `nm-proto` extensible             |
| Latence du socket       | Perception de fluidité de l'app via IPC       | Pagination par curseurs, mesures dès le squelette             |
| Signature des helpers   | Erreurs de notarisation découvertes tard      | Valider la chaîne complète avec le squelette « Hello World »  |
| Cycle de vie de l'agent | Agent obsolète après mise à jour              | Poignée de main de version, `launchctl kickstart -k` par l'app |
| Démarrage à la demande  | `launchctl kickstart` sur un agent enregistré par `SMAppService` non vérifié | À valider dans le squelette ; repli : `MachServices` et démarrage par recherche du service |
| App mise à la corbeille | macOS nettoie-t-il l'enregistrement de l'agent ? | À vérifier dans le squelette ; sinon, inoffensif et documenté |
| Keychain du noyau       | Accès au trousseau par un exécutable sans bundle ; invite d'autorisation après une mise à jour si l'exigence de signature n'est pas reconnue | À valider dans le squelette : création par le noyau, lecture après mise à jour sans invite |
| Priorité du noyau       | `ProcessType` `Background` limite CPU et entrées-sorties, au risque de ralentir l'app | Décidé : `Standard`, car l'app attend le noyau à chaque action, surtout quand le Mac est chargé (après un redémarrage, pendant une sauvegarde) |
| Accès web local         | Exposition du serveur HTTP                    | Écoute sur `127.0.0.1`, désactivé par défaut ; le reste est à décider quand on codera `nm-web`, module non prioritaire (section 9) |
| Accès au socket         | Un processus quelconque pilote le noyau       | Vérification de la signature du client, droits par client (section 9) |
| Envoi par un agent      | Exfiltration par injection de prompt          | Confirmation obligatoire dans l'app (section 9)                |
| Chiffrement au repos    | Lecture directe de la base et du brut par un code du même utilisateur | Non couvert en v1 ; SQLCipher et brut chiffré, clé dans le Keychain réservée au noyau, à évaluer |
| Architecture Intel      | Coût des binaires universels                  | Décider du support Intel avant le premier release             |

---

## 12. Feuille de route

La première étape est un squelette complet de bout en bout, car la signature, l'enregistrement de l'agent et le démarrage à la demande sont les parties les plus pénibles à découvrir tard.

1. **Squelette « Hello World »** : un noyau qui répond `ping` sur le socket, un CLI qui l'appelle, une app SwiftUI qui l'appelle aussi, le tout signé, notarisé et publié par la CI. Le squelette valide aussi le démarrage à la demande par `launchctl kickstart`, l'arrêt après inactivité et le comportement quand l'app est mise à la corbeille.
2. **`nm-core` + `nm-store`** : modèles et persistance, testés sans réseau.
3. **`nm-proto` (IMAP) + `nm-engine`** : une synchro de bout en bout pilotée par tests.
4. **`nm-api` + CLI** : le CLI sert de banc d'essai du contrat ; une opération pénible à exposer en CLI signale un contrat mal posé.
5. **App Swift** : interface fluide au-dessus du contrat stabilisé.
6. **TUI, MCP** : adaptateurs rapides une fois le contrat stable.
7. **Réglages utilisateur** : boutons d'installation du CLI et de la TUI, connexion MCP, tri en arrière-plan, désinstallation.
8. **Second temps : serveur web (`nm-web`)** et son bouton d'accès dans les réglages ; sa sécurité sera décidée à ce moment (section 9).
