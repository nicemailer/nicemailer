# NiceMailer : plan de travail du back-end IMAP

Statut : phases 0 à 8 terminées (novembre 2026) ; reste l'essai sur un compte réel, à faire par l'utilisateur.

Ce document n'est pas une spécification : c'est le plan de réalisation du back-end qui communique avec les serveurs IMAP, récupère les mails, synchronise les dossiers, trie les mails entrants et importe un compte. Les règles restent dans les spécifications citées ; quand ce plan tranche une question qu'elles laissaient ouverte, la décision est reportée dans la spécification concernée au moment où la phase qui l'implémente est livrée.

Méthode : **une phase = une revue = un commit**. Chaque phase livre son code, ses tests, la documentation de `docs/` et une commande CLI qui sert de banc d'essai du contrat.

## 1. Périmètre

| Inclus | Exclu de ce plan (phases suivantes) |
| --- | --- |
| Client IMAP, connexion et authentification par mot de passe | Envoi SMTP, brouillons, `append_draft`, `send` (`brouillons.md`) |
| Récupération complète : en-têtes, puis tous les corps en tâche de fond | Synchronisation de la configuration entre Mac (`synchronisation-configuration.md`) |
| Synchronisation des dossiers du workflow, dans les deux sens (spool) | Dossiers sans rôle : non gérés, choix assumé (`dossiers.md`) |
| Screening à la réception, authentification de l'expéditeur | OAuth2, sauf s'il se révèle facile (section 6) |
| Création d'un compte et import initial | Gmail et Microsoft 365 comme cibles de V1 (section 6) |
| Configuration `config.toml` | JMAP, LaunchAgent, vérification des signatures des clients |
| Robustesse : reprise, pannes réseau, ramasse-miettes du brut | Bridage du débit de téléchargement (à analyser plus tard) |

**Cible V1 : tout serveur IMAP4rev1 classique.** Le serveur de référence est celui de l'utilisateur, Infomaniak (un Dovecot). Avant authentification, il n'annonce que `IMAP4rev1 UIDPLUS IDLE LITERAL+ QUOTA AUTH=PLAIN AUTH=LOGIN` ; après authentification, il annonce notamment `CONDSTORE QRESYNC ESEARCH MOVE SPECIAL-USE LIST-STATUS UIDPLUS IDLE LITERAL+ ENABLE ID NAMESPACE BINARY STATUS=SIZE` (relevé du 5 novembre 2026). Le chemin principal est donc QRESYNC. Deux conséquences :

- les capacités sont **relues après l'authentification** (code `[CAPABILITY …]` de la réponse `OK` au LOGIN, sinon une commande `CAPABILITY`) ; celles d'avant ne servent qu'à choisir le mécanisme d'authentification ;
- les chemins de repli (sans CONDSTORE, sans MOVE, sans SPECIAL-USE) restent nécessaires pour les autres serveurs et restent testés (section 4), mais ils ne sont plus le cas de référence.

## 2. Décisions prises

| Sujet | Décision |
| --- | --- |
| Bibliothèque IMAP | `imap-codec` **2.0.0-alpha.10**, version épinglée, sous notre propre client async (tokio + `tokio-rustls`). Essai de la phase 0 : décode sans erreur CAPABILITY, LIST avec attributs d'usage spécial, SELECT, FETCH (UID, FLAGS, INTERNALDATE, RFC822.SIZE, MODSEQ, BODY[HEADER], BODY[]<partiel>), APPENDUID, COPYUID, ENABLE, VANISHED et VANISHED (EARLIER), STATUS. `async-imap` est écarté : ni QRESYNC ni VANISHED. Le risque d'une version alpha est contenu : seul le module `nm-proto::imap` voit les types d'`imap-codec`, le reste du noyau voit nos types |
| Gros messages | `imap-codec` garde chaque réponse entière en mémoire. Les messages au-delà d'un seuil (4 Mo) sont téléchargés par morceaux de 1 Mo (`BODY.PEEK[]<offset.longueur>`, IMAP4rev1 standard), écrits au fil de l'eau dans le stockage brut. La mémoire reste bornée quelle que soit la taille du message (`stockage-brut.md`, section 11) |
| TLS | rustls (fournisseur `ring`), certificat vérifié par macOS lui-même (`rustls-platform-verifier`) : même confiance que Mail et Safari, autorités ajoutées par l'utilisateur et révocation comprises ; fichier d'autorité facultatif par compte (`ca_file`), utile pour un serveur auto-hébergé et pour les tests. Jamais de désactivation de la vérification. macOS impose ses règles aux certificats de serveur, même d'une autorité approuvée (825 jours au plus, usage « authentification de serveur ») : le certificat de test les respecte (`make-tls.sh`) |
| Écriture des commandes | Écrites par NiceMailer (`encode.rs`), lues par `imap-codec` : ses types ne savent pas exprimer `LIST … RETURN (SPECIAL-USE)` ni `SELECT … (QRESYNC …)`. Toute valeur venue de l'extérieur passe par une seule fonction qui choisit atome, chaîne entre guillemets ou littéral : aucune ne peut terminer la ligne et injecter une commande |
| Corps | Tous téléchargés en tâche de fond : Inbox d'abord, puis les autres dossiers, les plus récents d'abord. Un message ouvert dont le corps manque est téléchargé en priorité |
| Déplacements faits ailleurs | Rapprochés par `Message-ID` quand c'est sans ambiguïté (section 5.4) : le message garde son `id`, ses tags, son historique et sa consultation |
| Sous-domaines | Inclus partout, y compris dans le routage à la réception : une décision sur `exemple.fr` s'applique à `news.exemple.fr` sans décision propre. `modele de donnees.md` sera corrigé (phase 6) |
| Lectures SQLite | Une connexion d'écriture unique et un pool de connexions de lecture (WAL) : une écriture de synchronisation ne fait plus attendre l'app |
| Secrets | **Trousseau dès le développement** (phase 1 bis, décision de l'utilisateur) : élément « mot de passe générique » du trousseau de session, le serveur (hôte, identifiant) dans un attribut, lisible sans autorisation ; le noyau seul lit et écrit, et n'affiche jamais de dialogue. Saisie par `nicemailer account password <clé>` (méthode `set_password`, refusée aux agents). Une clé `password` dans `config.toml` est une erreur. En développement, `make core` signe le noyau avec un certificat Apple Development (gratuit avec un identifiant Apple, créé une fois dans Xcode) : sans signature stable, chaque recompilation serait une autre application pour le trousseau, et perdrait l'accès aux mots de passe (mesuré). Un certificat auto-signé (`make dev-cert`, première solution) ne suffit pas : dans le trousseau de session, un élément reçoit une partition tirée de la signature de son créateur, `cdhash:<empreinte du binaire>` sans identifiant d'équipe Apple, `teamid:<équipe>` avec. Seule la seconde survit à une recompilation (constaté le 5 novembre 2026 ; un trousseau créé par `security create-keychain`, celui des tests, n'a pas de partition, d’où des tests verts). Remplace la lecture du mot de passe dans le fichier (feature `dev-passwords`) de la phase 1 |
| Serveur de test | Dovecot 2.4.5 sous OrbStack (`dev/dovecot/`), deux instances : complète (proche d'Infomaniak après authentification), et minimale (serveur IMAP4rev1 sans extension de synchronisation) |

Conseil pour le développement : garder les comptes de test dans un dossier à part, désigné par `NICEMAILER_CONFIG` (son `accounts.toml` est à côté), plutôt que dans `~/.config/nicemailer/`.

## 3. Analyse de l'existant

**Réutilisable** : schéma v3 et migrations ; stockage brut adressé par contenu ; analyse MIME (`ingest.rs`) ; threading et dédoublonnage de l'import mbox ; décisions de screening, rangement et annulation (`screening.rs`) ; pièces jointes ; reindex ; événements séquencés avec rejeu.

**À modifier** :

- `nm-daemon` partage une seule connexion SQLite derrière un `Mutex` : toutes les lectures attendent la fin de chaque écriture. Phase 1.
- `RawStore::write(&[u8])` prend le message entier en mémoire. Il faut un écrivain en flux (hash et compression au fil des morceaux). Phase 3.
- L'insertion d'un message (`Writer::insert`, `thread_for`, `address_id`) n'existe que dans `import.rs`, pour un message complet. Il faut un chemin commun à l'import mbox et à IMAP, en deux temps : en-têtes seuls (`body_state = 0`), puis message complet. Phase 3.
- Les commandes (`move_thread`, lu, drapeau, tags, rangements) ne modifient que la base. Sur un compte IMAP, elles doivent aussi mettre une action dans le spool. Phase 5.

**À créer** : la crate `nm-engine` (orchestration : connexions, synchronisation, spool, screening à la réception) ; le client IMAP de `nm-proto` ; la configuration ; la lecture d'`Authentication-Results` et la Public Suffix List.

## 4. Stratégie de test

Trois niveaux, pour chaque phase :

1. **Tests unitaires** sans réseau, à côté du code : analyse des réponses, algorithmes de synchronisation sur un faux serveur en mémoire, règles du screening.
2. **Tests d'intégration contre Dovecot**, dans les deux configurations : complète et minimale. Chaque test ouvre un utilisateur au nom aléatoire, donc une boîte vide, et dépose ses mails par `APPEND`. `make imap-up` démarre les serveurs, `make imap-test` lance ces tests ; `make test` ne les exige pas.
3. **Pannes injectées** : un mandataire TCP de test entre le client et Dovecot coupe la connexion au milieu d'un littéral, retarde ou tronque les réponses. Chaque coupure doit aboutir à un état cohérent, sans perte ni doublon.

Critère de « récupération parfaite » : la commande `nicemailer sync verify <compte>` (phase 4) compare, dossier par dossier, l'état du serveur et la base : mêmes UID, mêmes flags, chaque message complet avec un brut dont le hash est vérifié. Les tests d'intégration l'exécutent après chaque scénario.

## 5. Phases

### Phase 0 : essais et outillage — terminée

- Essai de `imap-codec` et de `async-imap` contre Dovecot ; décision de la section 2.
- `dev/dovecot/` : deux serveurs Dovecot, certificat signé par une autorité de test générée localement (`make-tls.sh`).
- `make imap-up`, `make imap-down`.
- Ce plan.

### Phase 1 : fondations — terminée

Documentation : `docs/configuration.md`. Écarts au plan : le trousseau dès cette phase, en phase 1 bis (section 2) ; `ConfigError` porte aussi `key` ; `config_status` donne l'état du fichier (règle « un état ne repose jamais sur un événement seul », `evenements.md`) ; schéma en version 4 (`account.disabled_at`, et `account.position`, l'ordre des comptes du sélecteur `@`, modifié par `set_account_order`).

Décision de l'utilisateur à la revue : les mails d'un compte désactivé restent visibles ; seule sa synchronisation s'arrête.


- Crate `nm-engine`, qui ne dépend d'aucune interface (`architecture.md`, principe 1).
- `config.toml` (`configuration.md`) : chemin et `NICEMAILER_CONFIG`, lecture, validation complète (fichier invalide refusé en entier, ligne fautive), clés inconnues signalées, réglages par compte, surveillance FSEvents (crate `notify`) et application sans redémarrage, `nicemailer config check | defaults | path`. Pas encore d'écriture par le contrat : elle arrive avec `add_account` (phase 7).
- Comptes : ligne `account` créée ou réactivée pour chaque clé du fichier ; clé disparue = compte désactivé, données conservées.
- Secrets : interface `Secrets`, implémentée par le trousseau (phase 1 bis). L'hôte et l'identifiant sont enregistrés avec le secret, pour refuser de l'envoyer à un autre serveur.
- SQLite : connexion d'écriture et pool de lecture ; les requêtes du contrat passent par le pool.
- Événements `AccountChanged`, `AccountError`, `ConfigChanged`, `ConfigError` ; `list_accounts`.

Tests : `configuration.md`, section 12, sauf l'écriture ; une lecture n'attend pas une écriture longue.

### Phase 1 bis : trousseau — terminée

Voir section 2, « Secrets », et `docs/configuration.md`, « Passwords ». Essai préalable sur un trousseau jetable : un binaire recompilé perd l'accès à ses éléments ; signé avec un même certificat personnel et un même identifiant, il le garde, sans que le certificat soit approuvé. Les attributs d'un élément se lisent sans autorisation, même trousseau verrouillé, d'où la vérification du serveur à chaque lecture du fichier sans risque de dialogue.

### Phase 2 : client IMAP (`nm-proto::imap`) — terminée

Documentation : `docs/imap.md`. Tests : `nm-proto/tests/imap_script.rs` (serveur scripté, dans `make test`), `nm-proto/tests/dovecot.rs` et `nm-daemon/tests/imap_rpc.rs` (les deux Dovecot, `make imap-test`).

Décisions et écarts au plan :

- **Lecture bornée** : une réponse de plus de 16 Mio est refusée sans être gardée en mémoire ; la session continue (le découpage d'`imap-codec` a trouvé sa fin). Corps entiers jusqu'à 4 Mio, au-delà par morceaux de 1 Mio ; l'arrêt se fait sur le premier morceau court, pas sur la taille annoncée.
- **Délais** sur chaque lecture et chaque écriture (90 s de silence), pas sur une commande entière : un gros téléchargement qui avance n'expire jamais.
- **Erreurs fatales ou non** : réseau, délai, TLS, `BYE` et désaccord sur l'état de la conversation ferment la session ; `NO`, `BAD`, extension absente et réponse illisible (`Malformed`) la laissent utilisable.
- **NUL dans un littéral** : interdit par la RFC 3501 et refusé par `imap-codec`, il bloquerait pour toujours la synchronisation sur le message. Il est lu comme 0x80, ce que Dovecot envoie à sa place.
- **Réponses non sollicitées** gardées dans l'ordre avec le dossier sélectionné à leur arrivée (`Notice`), jamais perdues.
- **`UTF8=ACCEPT` n'est pas activé** : Infomaniak ne l'annonce pas, et un seul codage des noms de dossiers (UTF-7 modifié) partout évite des écarts. Le nom brut reçu du serveur est celui qu'on lui renvoie, jamais un nom décodé puis recodé.
- **Déplacement sans `MOVE` ni `UIDPLUS`** : copie et `\Deleted`, mais pas d'`EXPUNGE`, qui effacerait aussi des messages marqués par un autre client.
- **`nicemailer imap probe`** passe par le noyau (méthode `probe_account`), seul à pouvoir lire le mot de passe. Il ne sélectionne aucun dossier et n'écrit rien. Il propose aussi le rôle de chaque dossier (`dossiers.md`, « A la création du compte »). Nouveaux codes d'erreur : `ACCOUNT_NOT_READY` (1008), `SERVER` (1009).
- **Pas encore** : envoi de plusieurs commandes à la suite sans attendre (pipelining, phase 8 si les mesures le demandent), `COMPRESS`, `BINARY`, `ESEARCH`.

Contenu livré :

- Connexion TLS directe ou STARTTLS (et `none` réservé à la machine locale), délais d'attente sur chaque lecture et écriture. STARTTLS sans repli en clair, capacités relues après la mise à niveau, données reçues avant la poignée de main refusées.
- CAPABILITY (avant et après authentification), LOGIN et AUTHENTICATE PLAIN (avec ou sans SASL-IR), ID, ENABLE (CONDSTORE, QRESYNC si annoncés), NAMESPACE, LIST (attributs d'usage spécial, délimiteur, noms en UTF-7 modifié ; avec LIST-STATUS si annoncé), CREATE, STATUS, SELECT et EXAMINE (avec QRESYNC si annoncé), UID FETCH (en-têtes complets, flags, date interne, taille, MODSEQ, corps entier ou par morceaux, CHANGEDSINCE et VANISHED), UID SEARCH, UID STORE (avec UNCHANGEDSINCE), UID MOVE ou COPY + STORE `\Deleted` + UID EXPUNGE, APPEND avec APPENDUID, IDLE (25 minutes au plus, RFC 2177), NOOP, LOGOUT.
- Réponses non sollicitées (EXISTS, EXPUNGE, FETCH, VANISHED, ALERT) remontées à l'appelant, jamais perdues.
- `nicemailer imap probe <compte>` : capacités, dossiers, rôles proposés, sans rien modifier.

Tests : chaque commande contre les deux Dovecot ; conversations scriptées (coupure au milieu d'un littéral, réponse illisible ou trop grosse, silence, étiquette inconnue, `BYE`, littéral refusé) ; coupure réseau par un relais TCP au milieu d'un corps, puis nouvelle session qui le récupère entier.

### Phase 3 : ingestion commune — terminée

- Écrivain en flux du stockage brut : morceaux successifs, hash et compression au fil de l'eau, vérification de la taille annoncée, temporaire supprimé en cas d'échec (`stockage-brut.md`, section 5).
- Chemin d'insertion commun à l'import mbox et à IMAP :
  - en-têtes seuls : ligne `message` (`body_state = 0`), adresses, références, threading, entrée plein texte sans corps, compteurs ;
  - message complet : brut, `message_body`, pièces jointes, désinscription, `body_state = 1`, plein texte complet.
- Threading indépendant de l'ordre d'arrivée (réponse avant l'original, fusion de deux conversations), comme pour l'import mbox.

Tests : les tests de l'import mbox restent verts ; un message de 150 Mo en morceaux, mémoire bornée ; disque plein simulé, aucune ligne sans fichier.

Réalisé (novembre 2026), détail dans `docs/ingestion.md` :

- `RawWriter` (`raw.rs`) : hash et compression au fil des morceaux, taille vérifiée à la fin si elle est donnée, temporaires supprimés dès que l'écriture n'aboutit pas (coupure, disque plein, erreur). `RawStore::write`, pour l'import mbox, est le même écrivain alimenté d'un seul morceau.
- Copie non compressée pour l'analyse : en mémoire jusqu'à 16 Mo, au-delà dans un temporaire projeté en mémoire (`mmap`), supprimé aussitôt. Mesure : 26 Mo de tas au plus pour un message de 150 Mo. Pas de nouvelle dépendance : l'appel `mmap` passe par `libc`.
- `Inserter` (`insert.rs`) : `insert` avec les en-têtes seuls (`ingest::parse_headers`) ou avec le corps, `complete` pour le corps (résultat `Done`, `Already` ou `Missing`), `commit` qui recalcule une fois par lot conversations et compteurs. L'import mbox passe par lui : même vitesse qu'avant (30 000 messages générés, 10 s dans les deux cas), mêmes conversations.
- Écart : le **parent fantôme** de JWZ est ajouté au threading. Deux réponses à un message absent (supprimé, ou rangé dans un dossier sans rôle, que NiceMailer ne synchronise pas) forment une conversation. Avant, elles restaient séparées tant que le parent n'arrivait pas. Les bases déjà importées ne sont pas reprises.
- Écart : une insertion n'ajoute pas le message au `ChangeSet` (pas d'événement `MessageChanged` par nouveau message, 50 000 pour un import) ; le dossier suffit, et la synchronisation aura `MessageArrived`. `complete` l'ajoute : le corps est arrivé.
- Le cache des adresses de l'`Inserter` ne garde que ce qui a été validé : après une transaction annulée (`abandon`), aucun identifiant de ligne disparue n'est réutilisé.
- Tests : threading sur les 120 ordres d'arrivée de cinq messages, lot annulé, disque plein simulé (`RawStore::with_disk_space`), message de 150 Mo à mémoire mesurée, et contre Dovecot (`make imap-test`) un corps de 22 Mo reçu en morceaux, puis coupé à 8 Mo sans rien laisser.
- Outillage : `sha2`, `mail-parser` et `zstd-sys` sont compilés optimisés même en debug, pour des tests rapides (24 s ramenées à 0,8 s).
- Reste pour la phase 4 : la conduite en cas d'écart entre `RFC822.SIZE` et les octets reçus (`stockage-brut.md`, section 5, étape 2), et l'appel de `RawStore::barrier` avant chaque validation de lot qui contient des corps.

### Phase 4 : synchronisation des dossiers (serveur vers base) — terminée

- **Connexions par compte** : une connexion IDLE sur l'Inbox ; une connexion de travail (synchronisation des dossiers, puis spool en phase 5) ; une connexion de téléchargement des corps, de basse priorité, pour que 200 000 corps à télécharger ne retardent ni une action ni l'arrivée d'un mail. Plafond configurable.
- **Liste des dossiers** : LIST, rôles déjà attribués, dossiers disparus ou renommés.
- **Synchronisation d'un dossier** (`modele de donnees.md`, section 5) :
  - UIDVALIDITY changé : purge du dossier, puis resynchronisation ;
  - avec QRESYNC : SELECT avec `(QRESYNC (uidvalidity highestmodseq))`, VANISHED et flags modifiés en une commande ;
  - avec CONDSTORE seul : flags modifiés par `CHANGEDSINCE`, suppressions par comparaison des UID ;
  - sans rien (serveur minimal) : nouveaux UID depuis `uidnext`, suppressions par `UID SEARCH ALL` comparé à la base, flags par `UID FETCH 1:* (FLAGS)`. Ce dernier coûte environ 40 octets par message, soit 4 Mo pour un dossier de 100 000 mails ; il est fait à chaque interrogation de l'Inbox, et moins souvent pour les gros dossiers de rangement (fréquence à mesurer) ;
  - nouvelles valeurs de `uidvalidity`, `uidnext`, `highestmodseq` enregistrées dans la même transaction que les messages.
- **Rapprochement des déplacements faits ailleurs** : un UID nouveau dans un dossier B, dont le `Message-ID` correspond à une seule ligne du même compte dans un dossier A, sans action en attente et de même taille, est un candidat. Les candidats sont vérifiés par dossier source, en une commande (`UID SEARCH UID <ensemble>`) : si l'UID d'origine n'existe plus dans A, la ligne est déplacée (`UPDATE folder_id, uid`) au lieu d'être supprimée puis recréée. Sinon, c'est une vraie copie et une nouvelle ligne est insérée. Pas de `Message-ID`, plusieurs candidats, ou des tailles différentes : pas de rapprochement.
- **Rythme** : IDLE sur l'Inbox ; autres dossiers toutes les `sync.poll_interval_secs` (300 s) ; `sync_now` pour forcer.
- **Téléchargement des corps** dans l'ordre de la section 2.
- **Pannes** : recul exponentiel par compte, plafonné ; après une veille du Mac (repérée par l'écart entre l'horloge continue et l'horloge d'éveil), la connexion est testée et gardée si elle répond ; sinon une seule reconnexion, une fois le Mac éveillé depuis 15 s (`docs/sync.md`, « Sleep of the Mac ») ; `AccountError` en cas d'erreur d'authentification, sans nouvelle tentative avant un changement de configuration (pour ne pas faire verrouiller le compte par le serveur).
- **Rythme des connexions** (ajouté le 8 novembre 2026, après le verrouillage d'un compte Orange) : au plus 10 connexions par compte et par heure glissante, les deux connexions comprises ; le recul ne revient à son minimum qu'après une connexion qui a tenu 10 minutes, pas après une passe réussie ; une connexion coupée par le serveur pendant IDLE n'est pas une erreur : IDLE est renouvelé à la moitié du temps constaté, sur une seule connexion, et si le serveur coupe encore, le compte est interrogé (une connexion par passe, toutes les 10 minutes au plus). Détail dans `docs/sync.md`, « Pace of the connections ».
- `SyncProgress`, `MessageArrived`, `FolderChanged`, `ThreadChanged`.
- Tant que la phase 5 n'est pas livrée, les commandes qui modifient des mails sont refusées sur les comptes IMAP (sinon la synchronisation suivante les déferait).
- `nicemailer sync status | now | verify`.

Tests : les deux Dovecot ; changements faits par une seconde connexion de test (« le téléphone ») : flags, suppressions, déplacements rapprochés ou non, UIDVALIDITY changé ; coupures à chaque étape ; `sync verify` vert après chaque scénario.

Réalisé (novembre 2026), détail dans `docs/sync.md` :

- `nm-engine/src/sync/` : un travailleur par compte de `config.toml`, sur ses propres fils d'exécution (un runtime tokio mono-fil par connexion : le réseau est asynchrone, les écritures en base restent des appels courts et bloquants). `nm-store/src/sync.rs` écrit ce que le serveur a montré, sans réseau : toutes ses règles sont testées sans serveur.
- **Lecture seule envers le serveur** : `EXAMINE` partout, `PEEK` pour toute lecture. La synchronisation ne peut rien changer sur le serveur, même par erreur, tant que le spool n'existe pas. Les commandes qui modifient un mail sont refusées sur ces comptes (code `1010`, `READ_ONLY`) ; une décision sur un expéditeur est enregistrée mais ne range aucun de leurs mails. Les faux comptes de `make seed` et les comptes d'archives restent modifiables : le noyau indique au `Store` les comptes de `config.toml`.
- Une passe en trois temps : découverte de tous les dossiers changés (suppressions, flags, nouveaux UID), puis arrivées, puis fin de chaque dossier. Les suppressions de toute la passe sont connues avant la première arrivée : un déplacement fait par le téléphone garde la ligne (même `id`, tags, historique), quel que soit l'ordre des dossiers.
- Écart : **deux connexions par compte** au lieu de trois. La connexion de travail attend en IDLE sur l'Inbox entre deux passes et en sort en quelques millisecondes quand elle a autre chose à faire ; une troisième ne ferait qu'occuper une des rares connexions permises par les serveurs. `sync.max_connections = 1` met aussi les corps sur la connexion de travail.
- Après un changement signalé par IDLE, la passe attend 1,5 s : un client qui déplace sans `MOVE` (copie, `\Deleted`, effacement) serait sinon vu au milieu de son déplacement, et le mail pris pour une copie. Mesuré sur le Dovecot minimal.
- Le dossier sélectionné par la session (l'Inbox, pour IDLE) est toujours ouvert à chaque passe : Dovecot répond au `STATUS` d'un dossier sélectionné avec ce que la session en sait déjà (constaté : les flags changés par le téléphone étaient manqués).
- Rôles des dossiers, provisoirement jusqu'à la phase 7 : la première synchronisation d'un compte sans aucun rôle applique la proposition de `folders.rs` (consignée dans `audit_log`). Un dossier renommé sur le serveur est reconnu à son `UIDVALIDITY` ; un dossier à rôle disparu est conservé avec ses mails, non synchronisé.
- Question ouverte 2 tranchée provisoirement : sans CONDSTORE, comparaison complète des flags de l'Inbox à chaque passe, des autres dossiers à chaque `poll_interval_secs`.
- Corps de taille différente de `RFC822.SIZE` : refusé, retenté au redémarrage seulement (`stockage-brut.md`, section 5).
- `MessageArrived` pour chaque nouveau mail, sauf pendant la première synchronisation d'un dossier ; `SyncProgress` est transitoire, sans `seq`. L'app ne prenait pas en compte l'absence de `seq` : elle y voyait un trou et rechargeait tout (corrigé, `AppModel.order`).
- `nicemailer sync status | now | verify`.
- Tests : `nm-store/tests/sync.rs` (11, sans serveur) ; `nm-daemon/tests/sync_rpc.rs` (6, `make imap-test`) : le vrai noyau contre les deux Dovecot, une seconde connexion jouant le téléphone (arrivée annoncée, flags, déplacement avec et sans `MOVE` gardant l'identité, suppression, commandes refusées), une seule connexion, `UIDVALIDITY` changé, mot de passe refusé sans nouvelle tentative, coupures réseau (connexion, liste, corps) sans perte, doublon ni fichier partiel ; `sync verify` vert après chaque étape.
- Reste pour plus tard : la reprise d'un brut corrompu ou absent depuis le serveur (phase 8), le GC des blobs des messages supprimés (phase 8).

### Phase 5 : spool (base vers serveur) — terminée

- Les commandes existantes créent leurs actions dans la même transaction que la modification locale (`modele de donnees.md`, section 6) : `move`, `set_flags`, tags (mots-clés), `delete`.
- Exécuteur sur la connexion de travail : fusion, exécution en série par message, regroupement en un `UID MOVE` par couple de dossiers, COPYUID, rattachement par `Message-ID` sans UIDPLUS ; erreur temporaire (recul) ou définitive (restauration depuis `undo`, actions suivantes annulées, `ActionReverted`) ; message disparu = succès ; reprise des actions `running` au démarrage ; purge.
- Interaction avec la synchronisation : UID d'une action en attente ignoré dans le dossier source, rattachement dans le dossier cible, flags locaux conservés tant que l'action n'est pas exécutée.
- Mots-clés refusés par le serveur (`PERMANENTFLAGS` sans `\*`) : tags et marques de retour restent locaux.
- Valeurs validées par l'utilisateur (5 novembre 2026) pour ce que la spécification laisse ouvert : 8 tentatives au plus, recul de 30 s doublé à chaque échec et plafonné à 1 h, purge des actions terminées après 7 jours.

Tests : `modele de donnees.md`, section 6, scénario par scénario ; deux noyaux qui déplacent le même mail ; plantage simulé entre le MOVE et le passage à `done`.

Réalisé (novembre 2026), détail dans `docs/spool.md` :

- Les commandes passent toutes par trois points de `nm-store` (`move_message`, la fin des commandes de flags, `tag_thread`), qui écrivent l'action dans la transaction de la modification : déplacements, rangements après une décision, annulation (`z`), lu, drapeau, tags. Seuls les comptes de `config.toml` ont des actions (table temporaire `server_account` de la connexion d'écriture) ; les faux comptes de `make seed` et les comptes d'archives restent locaux. Le refus de la phase 4 (code `1010`) disparaît.
- `delete` n'est pas créé : aucune commande ne supprime un mail du serveur ; « supprimer » est un déplacement vers Deleted.
- Exécuteur sur la connexion de travail, avant chaque passe et dès qu'une commande crée une action (le travailleur sort d'IDLE). `SELECT` pour lui seul ; la synchronisation garde `EXAMINE`.
- Écart : une action `set_flags` retient le dossier et l'UID du message au moment où le flag change. Sans cela, un tag suivi d'un déplacement s'attendaient mutuellement (le tag cherchait l'UID que seul le déplacement, placé après lui, donne) : interblocage constaté par le test contre Dovecot.
- Un refus (`NO`) d'un `UID MOVE` est vérifié : les UID partis entre la recherche et le déplacement (un autre Mac) sont traités comme « message disparu », pas comme un échec. Constaté avec deux noyaux sur la même boîte.
- Deux déplacements simultanés par deux Mac peuvent laisser deux copies sur le serveur (chaque `MOVE` voit le mail avant que l'autre l'efface) : aucun client ne peut l'empêcher ; les deux bases reflètent alors le serveur.
- La synchronisation ignore désormais une arrivée dont l'UID est déjà connu dans le dossier (message déplacé par le spool avec son `COPYUID`) : sans cela, insertion en double refusée par `UNIQUE (folder_id, uid)`, constatée sur le Dovecot complet.
- Correction de la phase 4 : les changements vus sur le serveur (déplacements faits ailleurs, suppressions, purge après `UIDVALIDITY`) ne sont plus écrits dans `audit_log`, conformément à `tracabilite.md` ; seule l'attribution de rôle d'un dossier y reste.
- Valeurs appliquées telles que validées : 8 tentatives, 30 s doublées, plafond d'1 h, purge après 7 jours. Remarque : avec 8 tentatives, le plus long délai est de 32 min ; le plafond d'1 h n'est jamais atteint, et l'ensemble des tentatives tient en une heure environ.
- L'app affiche `ActionReverted` dans sa barre d'état ; pas de scénario d'interface : les scénarios tournent sur les faux comptes, sans serveur, où rien ne peut être refusé.
- Limite : un serveur sans `MOVE` ni `UIDPLUS` ne peut effacer que les messages déplacés ; ils restent marqués `\Deleted` dans le dossier source jusqu'à ce qu'un autre client les efface.
- Tests : `nm-store/tests/spool.rs` (12, sans serveur) ; dans `nm-daemon/tests/sync_rpc.rs` (9 en tout, `make imap-test`), lu, tag et déplacement arrivant sur les deux serveurs, déplacement vers un dossier disparu annulé et annoncé, deux noyaux sur le même mail, plantage juste après le `MOVE` rejoué au redémarrage.

### Phase 6 : screening à la réception — terminée

- Lecture d'`Authentication-Results` (RFC 8601), choix de l'en-tête de confiance, traduction en verdict, alignement par domaine organisationnel (Public Suffix List embarquée, crate `psl` ou liste versionnée comme `public-domains.txt`) ; détection d'`authserv_id` sur les 50 derniers mails de l'Inbox (`authentification-expediteur.md`).
- Algorithme de routage (section 4 du même document), conversation connue, `auth_seen`, `auth_suspect`, ligne `screen` dans `audit_log`, déplacement par le spool, `uid_screening_start`.
- Retour d'une conversation de Todo ou Waiting dans l'Inbox, marques `$NmWasTodo` et `$NmWasWaiting`, retrait de la marque quand un mail marqué quitte l'Inbox ailleurs (`dossiers.md`).
- Mails découverts dans Sent : décision `inbox` pour les destinataires sans décision (`sent_to`), rangement de la conversation selon la présélection pour une réponse envoyée ailleurs.
- Correction de `modele de donnees.md` sur les sous-domaines.

Tests : `authentification-expediteur.md`, section 8 ; `dossiers.md` (retour de Todo/Waiting, mail placé ailleurs, mail de l'utilisateur) ; mail arrivé pendant la synchronisation initiale.

Réalisé (novembre 2026), détail dans `docs/screening.md` :

- `nm-store/src/auth.rs` (pur) : lecture des en-têtes `Authentication-Results` (commentaires imbriqués, repliement, guillemets), choix de l'en-tête de confiance, traduction en verdict, alignement par domaine organisationnel. Public Suffix List officielle embarquée telle quelle (`data/public-suffix/`, MPL 2.0, version du 4 novembre 2026, sections ICANN et privée), lue par 40 lignes de code : pas de nouvelle dépendance.
- `nm-store/src/reception.rs`, dans la transaction d'insertion des arrivées : verdict de tout mail reçu, `auth_seen` ; screening des arrivées de l'Inbox à partir de `uid_screening_start` (algorithme de la section 4, `auth_suspect`, ligne `screen`, déplacement par le spool de source `core`) ; retour d'une conversation de Todo ou Waiting avec `$NmWasTodo` / `$NmWasWaiting` ; mails découverts dans Sent (décision `inbox` des destinataires sans décision, raison `sent_to`, rangement de leurs mails de Quarantine ; rangement de la conversation d'une réponse selon la présélection, marques retirées).
- Retrait des marques : par `move_thread` (déjà local, désormais envoyé au serveur par le spool) et quand la synchronisation voit un mail marqué quitter l'Inbox ailleurs.
- Provisoire jusqu'à la phase 7 : `uid_screening_start` prend le `UIDNEXT` de l'Inbox à sa première synchronisation (et après un changement d'`UIDVALIDITY`) : les mails déjà là ne sont pas triés, les suivants le sont. Un compte déjà synchronisé en phase 4 reçoit sa frontière à la passe suivante.
- Détection d'`authserv_id` : sur les 50 UID les plus hauts de l'Inbox, quand l'Inbox a de nouveaux mails, au plus une fois par heure tant qu'aucun n'est trouvé ; la valeur détectée (ou celle du fichier) est montrée par `nicemailer sync status`.
- Un mail de l'utilisateur lui-même (son adresse ou une identité) reste dans l'Inbox (règle `self`) et ne fait revenir aucune conversation.
- Un dossier cible absent du compte (Quarantine, Newsletter… que la phase 7 créera) : le mail reste dans l'Inbox, la ligne `screen` le dit (`folder_missing`).
- Correction de la phase 5 : une passe qui crée des actions (screening, retour) les envoie aussitôt, sans attendre la passe suivante ; constaté par le test contre Dovecot.
- Correction de `modele de donnees.md` sur les sous-domaines.
- Tests : `auth.rs` (6 : table de traduction, faux en-tête, autre `authserv-id`, `From` absent ou multiple, suffixes publics et exceptions, seuil de 90 %) ; `nm-store/tests/reception.rs` (10 : exemples de la section 4, règles d'adresse et de domaine, conversation connue, frontière d'UID, retour de Todo/Waiting et ses exclusions, Sent, marque retirée ailleurs) ; contre Dovecot, un scénario complet (détection, Quarantine, faux en-tête, retour de Todo marqué sur le serveur, envoi depuis le téléphone qui sort un expéditeur de Quarantine).
- Limites : les domaines internationalisés sont comparés tels qu'écrits (pas de conversion punycode) ; question ouverte 1 de `authentification-expediteur.md` (Microsoft sans `authserv-id`) inchangée.

### Phase 7 : création d'un compte et import initial — terminée

- Commandes du contrat et CLI : `test_account` (connexion de test), `propose_folder_roles` (attributs d'usage spécial, sinon noms connus en plusieurs langues), `set_folder_roles` (avec création des dossiers manquants), `add_account` (écriture de `config.toml` par `toml_edit`, secret dans le Keychain), `account rename`, `account remove`.
- Trousseau : fait en phase 1 bis ; reste la suppression de l'élément par `account remove`, et son renommage par `account rename`.
- Import initial (`import-initial.md`) : comptage, synchronisation des en-têtes de Sent et correspondants, décompte exact par `UID SEARCH`, action `initial_archive` par lots de 1 000 UID, frontière `uid_screening_start`, UIDVALIDITY changé pendant l'import.

Tests : `import-initial.md`, section 11, contre les deux Dovecot ; `configuration.md` pour l'écriture du fichier.

Réalisé (novembre 2026), détail dans `docs/accounts.md` :

- Contrat : `test_account` (sonde, comptage de l'Inbox et de son `UIDNEXT`, nombre de correspondants lus dans les destinataires de Sent, offre d'archivage et décompte des valeurs par défaut), `count_archive` (décompte exact par le même `UID SEARCH` que l'archivage), `add_account`, `set_folder_roles`, `rename_account`, `remove_account`. Toutes refusées aux agents (`1007`). CLI : `nicemailer account add | roles | rename | remove`.
- Écart : pas de méthode `propose_folder_roles` distincte. La proposition est rendue par `test_account` (et `nicemailer imap probe` pour un compte existant) ; `set_folder_roles` l'applique.
- `add_account`, dans l'ordre : dossiers manquants créés sur le serveur (nom anglais du rôle, dans l'espace de noms personnel), compte, rôles, frontière `uid_screening_start` (le `UIDNEXT` lu par `test_account`), décisions des correspondants, action `initial_archive`, mot de passe dans le trousseau, puis `config.toml` (ajout par `toml_edit`, relu avant, écrit atomiquement, validé), appliqué aussitôt.
- Archivage côté serveur : exécuté par le spool avant toute passe ; la passe n'ouvre pas l'Inbox tant qu'il reste à faire. Recherche refaite à chaque reprise, déplacement par lots de `import.batch_size`, une ligne `initial_archive` au journal. Interruption testée en tuant le noyau après le premier lot.
- Date par défaut des mails lus : le lendemain (`BEFORE` exclut le jour donné), pour archiver « tous » les lus comme le veut la section 3 de `import-initial.md`.
- Adresses découvertes par la première synchronisation d'un dossier : `imported_from = 'imap_initial'`.
- Les rôles et la frontière provisoires des phases 4 et 6 restent la règle d'un compte déclaré à la main dans `config.toml`.
- `account rename` : base, trousseau, puis fichier (les tables gardent leur place). `account remove` : arrêt du travailleur attendu, fichier, données, trousseau ; le serveur n'est pas touché.
- Correction en route : `toml_edit` range un commentaire de fin de fichier (ou un fichier fait seulement de commentaires) après toute table ajoutée ; il est désormais placé avant le nouveau compte.
- Tests : `config_write.rs` (5), `setup.rs` du moteur (1), `nm-store/tests/setup.rs` (5), trousseau (1) ; contre Dovecot (`nm-daemon/tests/setup_rpc.rs`, 4) : petite Inbox laissée telle quelle et correspondants acceptés, mail arrivé pendant l'écran trié, fichier écrit avec son commentaire, prochain mail d'un expéditeur importé en Quarantine ; archivage conforme au décompte, mails marqués d'un drapeau laissés ; archivage interrompu repris sans perte ni doublon ; renommage avec données et mot de passe, suppression ; agent refusé. Le changement d'`UIDVALIDITY` pendant l'import est couvert sans serveur (la purge remet la frontière à zéro, la passe la reprend au nouveau `UIDNEXT`).
- Hors périmètre de la phase : l'écran de création dans l'app (la CLI le remplace pour l'instant).

### Phase 8 : robustesse et volume — terminée (essai réel à faire)

- Ramasse-miettes des blobs et balayage des orphelins (`stockage-brut.md`, section 7), devenus nécessaires puisque la synchronisation supprime des messages.
- Brut corrompu ou absent : mise à l'écart, puis retéléchargement depuis le serveur.
- Passage du stockage brut à deux niveaux de sous-dossiers (au-delà de 100 000 messages), par une migration.
- Essai de volume : boîte Dovecot générée de 100 000 mails ; durée de la synchronisation initiale, mémoire, réactivité des listes pendant la synchronisation (`make bench`).
- Essai sur le compte Infomaniak réel de l'utilisateur, en lecture d'abord, avec son accord explicite.

Réalisé (novembre 2026) :

- **Comptes en lecture seule** (`read_only = true`, `account add --read-only`), préalable à l'essai réel demandé par l'utilisateur sur une adresse Orange en production « où il ne faut rien toucher » : la session IMAP refuse elle-même `SELECT`, `STORE`, `COPY`, `MOVE`, `EXPUNGE`, `APPEND` et `CREATE` avant envoi ; pas de spool ; commandes refusées (`1010`) ; screening « à blanc » journalisé (`dry_run`) ; aucun dossier créé ni archivage. Vérifié contre Dovecot par un instantané du serveur avant et après. `test_account`, `count_archive` et `sync verify` n'écrivent jamais, quel que soit le compte.
- **Ramasse-miettes et balayage des orphelins** (`nm-store/src/gc.rs`), avec des épingles pour la concurrence avec l'ingestion (`stockage-brut.md`, section 7) ; une minute après le démarrage puis toutes les heures, et `nicemailer raw gc`.
- **Brut corrompu ou absent** : trouvé à la lecture (pièce jointe, export) ou par `sync verify --repair` ; mis à l'écart dans `.corrupt/`, messages repassés en en-têtes seuls et retéléchargés ; code `1011` (`RAW_DAMAGED`) ; un mail sans serveur garde son erreur.
- Écart : **pas de second niveau de sous-dossiers**. Mesuré : trouver un fichier coûte le même temps avec 400 et 40 000 fichiers par dossier ; la migration déplacerait tout le stockage pour rien (`stockage-brut.md`, section 7).
- **Essai de volume** (`make volume-test`, 100 000 mails générés sur le Dovecot de test, même machine) : en-têtes de tout en 20 s, corps compris en 728 s (environ 140 corps par seconde), mémoire du noyau au plus 289 Mo, liste de la boîte de réception pendant la synchronisation 2 ms en moyenne et 5 ms au pire, base 807 Mo pour 390 Mo de bruts (`docs/sync.md`, « Volume »). Trouvé en chemin : un trousseau de test se verrouille après 5 minutes d'inactivité, ce qui faisait échouer un essai long ; les trousseaux de test n'ont plus de délai.
- Reste : l'essai sur un compte réel, en lecture seule, que l'utilisateur fera lui-même (son mot de passe ne passe jamais par la conversation) ; procédure dans `docs/accounts.md`, « Trying NiceMailer on a mailbox in production ».

## 6. Gmail, Microsoft 365 et OAuth2 : ce que cela implique

| | Gmail | Microsoft 365, Outlook.com |
| --- | --- | --- |
| Authentification | Mot de passe d'application possible avec la validation en deux étapes ; sinon OAuth2 (XOAUTH2) | OAuth2 obligatoire : l'authentification de base IMAP est coupée |
| Coût d'OAuth2 | Les scopes mail sont « restreints » : une diffusion publique exige la vérification de Google et un audit de sécurité annuel payant. Sans eux, l'app reste en mode test, limitée à 100 utilisateurs déclarés | Inscription d'une application Azure, sans audit ; le consentement d'un administrateur peut être exigé par l'organisation |
| Dossiers | Ce sont des libellés : un message peut figurer dans plusieurs dossiers, et `[Gmail]/All Mail` contient tout. Archiver revient à retirer le libellé Inbox. Le modèle « un mail, un dossier de travail » fonctionne si `[Gmail]/All Mail` n'est pas géré, mais la suppression a un sens particulier (`[Gmail]/Trash`) | Dossiers classiques |
| Extensions | CONDSTORE, MOVE, UIDPLUS, `X-GM-MSGID` (identifiant stable, idéal pour le rapprochement des déplacements). Pas de QRESYNC | MOVE, UIDPLUS ; ni CONDSTORE ni QRESYNC |
| Authentification de l'expéditeur | `Authentication-Results` avec `authserv-id` `mx.google.com` : conforme | `authserv-id` souvent absent (`authentification-expediteur.md`, section 9) : règle propre au fournisseur nécessaire |
| Limites | Débit et volume quotidien de téléchargement par compte | Limitation de débit agressive |

Conclusion : le client IMAP est écrit pour accueillir un mécanisme SASL de plus (XOAUTH2, OAUTHBEARER) sans changement de structure. OAuth2 lui-même n'est pas « facile » : le flux d'autorisation (écoute sur `127.0.0.1`, renouvellement des jetons, Keychain) prendrait environ une phase, et la diffusion pour Gmail se heurte à la vérification de Google. Il est donc reporté après la V1. Gmail par mot de passe d'application est probablement utilisable dès la V1, à vérifier sur un compte de test.

## 7. Questions ouvertes

1. ~~Capacités d'Infomaniak après authentification~~ : réglé, voir section 1.
2. **Fréquence de la comparaison complète des flags** sur un serveur sans CONDSTORE, pour les gros dossiers hors Inbox : provisoirement à chaque `poll_interval_secs` (phase 4) ; à mesurer sur un vrai serveur sans CONDSTORE.
3. **Infomaniak et `Authentication-Results`** : quel `authserv-id`, et le serveur retire-t-il les en-têtes entrants qui portent le sien ? À vérifier sur quelques mails reçus.
