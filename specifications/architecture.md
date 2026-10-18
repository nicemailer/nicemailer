# NiceMailer – Document d'architecture

*Version du 2026-10-16*

NiceMailer est un client mail pour macOS : un front SwiftUI, un noyau Rust réutilisable par quatre autres interfaces (CLI, TUI, web, MCP), et une installation qui reste un simple double-clic pour le grand public.

---

## 1. Contexte et objectifs

L'application est découpée en deux parties : le front en Swift pour l'interface Mac, le back en Rust pour toute la logique mail. Le noyau Rust ne doit pas être pensé comme la bibliothèque d'une app, mais comme un produit à part entière dont l'app Mac n'est que le premier client.

### Objectifs

- Une seule logique métier (synchro, threading, recherche, envoi), partagée par toutes les interfaces.
- Cinq interfaces dès la conception : app macOS, CLI, TUI, web local et serveur MCP.
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
3. **Aucune logique métier côté Swift.** Si le front décide du threading, des règles de tri ou des brouillons, cette logique sera dupliquée dans les autres interfaces. Swift fait de l'affichage et de l'interaction.
4. **Un seul propriétaire de l'état.** Un unique processus noyau possède la base SQLite et la synchro ; tous les clients lui parlent.
5. **Vues prêtes à afficher.** Le contrat expose des structures de présentation (`ThreadSummary`, `MessageView`) plutôt que des lignes brutes de la base.
6. **Pagination partout.** Jamais de « charge tout » : une fenêtre et un curseur, pour tenir la promesse de fluidité.
7. **Complexité cachée.** Tout ce qui est technique (socket, CLI, MCP) s'active depuis l'interface, sans terminal.

---

## 3. Vue d'ensemble

Un seul processus noyau, démarré par launchd à l'ouverture de session et actif en continu, héberge le moteur ; l'app et toutes les autres interfaces sont des clients du même socket Unix.

```
 ┌───────────────┐ ┌─────────┐ ┌─────────┐ ┌──────────────┐ ┌─────────────┐
 │ App SwiftUI   │ │   CLI   │ │   TUI   │ │ nm-server    │ │  nm-mcp     │
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
| `nm-server` | Passerelle HTTP/WebSocket locale pour l'interface web                    | `nm-api`                          |
| `nm-mcp`    | Serveur MCP, proxy stdio vers le socket                                  | `nm-api`                          |

```
nicemailer/
├── crates/
│   ├── nm-core/  nm-proto/  nm-store/  nm-engine/  nm-api/
│   └── nm-daemon/  nm-cli/  nm-tui/  nm-server/  nm-mcp/
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
| Événements | noyau vers clients, notifications | `MessageArrived`, `SyncProgress`, `FlagsChanged`, `AccountError`, `ActionReverted`, `SendConfirmationRequested` |

### Règles du contrat

- **Pagination par curseurs.** Toute liste retourne une fenêtre et un curseur ; aucune requête ne renvoie une boîte entière.
- **Vues de présentation.** Les réponses sont des structures prêtes à afficher, indépendantes du schéma SQLite.
- **Événements en notifications JSON-RPC.** Les clients s'abonnent après la connexion ; l'app Swift les consomme via un `AsyncStream`.
- **Poignée de main de version.** À la connexion, le client annonce sa version de protocole ; si elle est trop ancienne, le noyau répond par une erreur explicite plutôt que de planter.
- **Erreurs typées.** Les codes d'erreur JSON-RPC sont stables et documentés, pour que le CLI et le MCP produisent des messages utiles.

### Pourquoi JSON-RPC

Le format est lisible et débogable à la main, le serveur MCP en est presque un simple adaptateur, et l'outillage Swift reste léger. gRPC serait plus rigoureux mais alourdirait la chaîne pour un gain faible ici.

### Exemple d'échange

```json
→ {"jsonrpc":"2.0","id":1,"method":"list_threads","params":{"folder":"inbox","limit":50,"cursor":null}}
← {"jsonrpc":"2.0","id":1,"result":{"items":[ /* ThreadSummary… */ ],"next_cursor":"c_8f2a"}}
← {"jsonrpc":"2.0","method":"MessageArrived","params":{"thread_id":"t_913","folder":"inbox"}}
```

---

## 6. Processus et packaging macOS

Le noyau est un LaunchAgent embarqué dans le bundle, enregistré par l'app via `SMAppService.agent(plistName:)` ; launchd le démarre à l'ouverture de session et le relance s'il s'arrête. Il tourne en tâche de fond, que l'app soit ouverte ou non : l'app, le CLI et le TUI en sont de simples clients.

### Options étudiées

| Option                        | Principe                                                                    | Verdict                                                                   |
| ----------------------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| A. Moteur dans l'app          | Le noyau tourne dans l'app, socket ouvert tant qu'elle est lancée           | Simple, mais CLI/MCP/web inutilisables app fermée, pas de synchro en fond |
| **B. Agent de fond embarqué** | Noyau dans le bundle, enregistré comme agent ; tous les fronts sont clients | **Retenue**                                                               |
| C. Démon installé à part      | Service séparé (Homebrew, pkg)                                              | Exclut le grand public                                                    |
| D. Hybride FFI + socket       | App liée au noyau via UniFFI, socket pour les autres                        | Deux chemins, comportements qui divergent                                 |

### Structure du bundle

```
NiceMailer.app/Contents/
├── MacOS/NiceMailer                                  ← app SwiftUI (client)
├── Library/LaunchAgents/eu.nicemailer.core.plist     ← déclare le noyau et le socket
└── Helpers/
    ├── nicemailer-core                               ← noyau Rust (nm-daemon)
    ├── nicemailer                                    ← CLI
    ├── nicemailer-tui                                ← TUI
    └── nicemailer-mcp                                ← serveur MCP (proxy stdio)
```

### Points techniques

- **LaunchAgent plutôt que LoginItem.** Un exécutable Rust nu s'enregistre mal comme LoginItem, qui attend un mini-bundle d'app.
- **Démarrage et relance.** Le plist déclare `RunAtLoad` et `KeepAlive` : le noyau démarre à l'ouverture de session et est relancé après un arrêt ou un plantage. Il déclare aussi le socket Unix (clé `Sockets`), qui relance le noyau si une connexion arrive alors qu'il est arrêté. Un `nicemailer list` fonctionne donc même si l'app est fermée.
- **Emplacement du socket.** `~/Library/Application Support/NiceMailer/`, droits `0600`, chemin court (les sockets Unix sont limités à environ 104 caractères). Les clients sont authentifiés par leur signature (section 9).
- **Versions alignées.** App, noyau et helpers sont livrés ensemble dans un seul bundle, ce qui évite les incompatibilités ; la poignée de main de version couvre les cas résiduels (ancien CLI installé, par exemple).

---

## 7. Expérience utilisateur non technique

L'utilisateur n'installe qu'une chose, l'app ; tout le reste s'active depuis les Réglages, sans terminal.

| Fonction | Ce que voit l'utilisateur | Ce qui se passe |
| --- | --- | --- |
| Premier lancement | Une seule autorisation système | L'app enregistre l'agent via `SMAppService` |
| Synchro en fond | Rien à configurer : les mails sont reçus app fermée | Le noyau tourne en permanence ; seul l'utilisateur peut le désactiver dans les réglages système (éléments d'ouverture) |
| CLI | Bouton « Installer la commande `nicemailer` » | Lien symbolique dans le PATH |
| MCP | Bouton « Connecter à Claude » | Écriture de l'entrée dans la config de Claude Desktop, ou bloc JSON à copier |
| Web | Bouton « Ouvrir dans le navigateur » | Serveur sur `127.0.0.1` avec jeton aléatoire, désactivé par défaut |
| Mises à jour | Notification Sparkle | Remplacement de tout le bundle, noyau compris |

---

## 8. Stockage et concurrence

Le noyau est l'unique propriétaire de la base SQLite ; aucun client n'y accède directement.

- **SQLite en mode WAL** : lectures concurrentes fluides pendant les écritures de synchro.
- **Corps de messages** : table séparée, lue à la demande pour garder les listes légères. C'est une interprétation du message brut, réécrite quand l'analyseur évolue ; seul le brut sur disque est immuable.
- **Recherche plein texte** : index FTS dans `nm-store`, interrogé via la requête `search`.
- **Runtime async unique** (tokio) côté noyau : les clients ne gèrent aucun thread réseau.
- **Pas d'accès partagé à la base** : l'app Swift, le CLI et le MCP passent tous par le noyau, ce qui supprime les conflits de verrouillage.

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
     and identifier in ("eu.nicemailer.app", "eu.nicemailer.cli", "eu.nicemailer.tui",
                        "eu.nicemailer.mcp", "eu.nicemailer.server")
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
| MCP (agent) | oui     | désactivé par défaut, activable dans les réglages de l'app | **toujours soumis à confirmation** | non                 |

Le serveur web est un client signé du socket. Il authentifie le navigateur par un jeton aléatoire, propre à la session, et il est le seul client à utiliser un jeton. Pour le socket lui-même, un jeton ne servirait à rien : il serait stocké dans un fichier `0600` que le code malveillant visé peut lire aussi.

### Confirmation des envois demandés par un agent

Un mail entrant est un contenu hostile, lu par un LLM qui dispose d'outils. Sans confirmation, une instruction cachée dans un mail suffirait à exfiltrer des données par un envoi. Le noyau applique donc la règle suivante à toute commande `send_message` venant du client MCP :

1. Le message est mis en attente de confirmation. Il n'entre pas dans la file d'envoi.
2. Le noyau émet l'événement `SendConfirmationRequested`. Si l'app est fermée, le noyau la lance.
3. L'app affiche le message complet (destinataires, objet, corps, pièces jointes) avec « Envoyer » et « Refuser ».
4. Seule l'app peut confirmer : la commande `confirm_send` est refusée à tout autre client, MCP compris.
5. Sans réponse après un délai (paramètre à fixer), la demande expire et vaut refus.
6. L'agent reçoit le résultat (`sent`, `refused` ou `expired`) en réponse à sa commande.

**Limite assumée** : un agent qui dispose aussi d'un accès au terminal peut appeler `nicemailer send` et contourner cette confirmation, puisque le CLI a le droit d'envoyer. La confirmation protège le canal MCP, pas l'utilisateur contre un agent auquel il a confié son terminal.

### Journal et limites

- Les actions des clients autres que l'app (CLI, TUI, web, MCP) sont journalisées et consultables dans l'app : client, action, message concerné, date.
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
| Cycle de vie de l'agent | Agent obsolète après mise à jour              | Poignée de main de version, redémarrage de l'agent par l'app  |
| Accès web local         | Exposition du serveur HTTP                    | Écoute sur `127.0.0.1`, jeton aléatoire, désactivé par défaut |
| Accès au socket         | Un processus quelconque pilote le noyau       | Vérification de la signature du client, droits par client (section 9) |
| Envoi par un agent      | Exfiltration par injection de prompt          | Confirmation obligatoire dans l'app (section 9)                |
| Chiffrement au repos    | Lecture directe de la base et du brut par un code du même utilisateur | Non couvert en v1 ; SQLCipher et brut chiffré, clé dans le Keychain réservée au noyau, à évaluer |
| Architecture Intel      | Coût des binaires universels                  | Décider du support Intel avant le premier release             |

---

## 12. Feuille de route

La première étape est un squelette complet de bout en bout, car la signature, l'enregistrement de l'agent et la socket activation sont les parties les plus pénibles à découvrir tard.

1. **Squelette « Hello World »** : un noyau qui répond `ping` sur le socket, un CLI qui l'appelle, une app SwiftUI qui l'appelle aussi, le tout signé, notarisé et publié par la CI.
2. **`nm-core` + `nm-store`** : modèles et persistance, testés sans réseau.
3. **`nm-proto` (IMAP) + `nm-engine`** : une synchro de bout en bout pilotée par tests.
4. **`nm-api` + CLI** : le CLI sert de banc d'essai du contrat ; une opération pénible à exposer en CLI signale un contrat mal posé.
5. **App Swift** : interface fluide au-dessus du contrat stabilisé.
6. **TUI, serveur web, MCP** : adaptateurs rapides une fois le contrat stable.
7. **Réglages utilisateur** : boutons d'installation du CLI, connexion MCP, accès web, synchro en fond.
