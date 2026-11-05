# NiceMailer : plan de travail du back-end IMAP

Statut : en cours. Phase 0 terminée le 2026-11-05.

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
| TLS | rustls, racines du système ; fichier d'autorité facultatif par compte (`ca_file`), utile pour un serveur auto-hébergé et pour les tests. Jamais de désactivation de la vérification |
| Corps | Tous téléchargés en tâche de fond : Inbox d'abord, puis les autres dossiers, les plus récents d'abord. Un message ouvert dont le corps manque est téléchargé en priorité |
| Déplacements faits ailleurs | Rapprochés par `Message-ID` quand c'est sans ambiguïté (section 5.4) : le message garde son `id`, ses tags, son historique et sa consultation |
| Sous-domaines | Inclus partout, y compris dans le routage à la réception : une décision sur `exemple.fr` s'applique à `news.exemple.fr` sans décision propre. `modele de donnees.md` sera corrigé (phase 6) |
| Lectures SQLite | Une connexion d'écriture unique et un pool de connexions de lecture (WAL) : une écriture de synchronisation ne fait plus attendre l'app |
| Secrets en développement | Mot de passe en clair dans `config.toml` (clé `password` du serveur IMAP), **lu seulement par un noyau compilé avec la feature `dev-passwords`** (`make core` l'active ; `cargo build` et le noyau distribué non). Un noyau sans la feature ne lit pas la clé et le dit dans l'erreur du compte (`AccountError`), et toute lecture du fichier avertit de sa présence. Choix revu en phase 1 : `cfg(debug_assertions)` aurait refusé le noyau de `make core`, compilé en `--release` ; et une `ConfigError` aurait refusé tout le fichier pour un problème qui ne concerne qu'un compte. Le Keychain arrive avec `add_account` (phase 7), derrière la même interface |
| Serveur de test | Dovecot 2.4.5 sous OrbStack (`dev/dovecot/`), deux instances : complète (proche d'Infomaniak après authentification), et minimale (serveur IMAP4rev1 sans extension de synchronisation) |

Conseil pour le développement : garder les comptes de test et leurs mots de passe dans un fichier à part, désigné par `NICEMAILER_CONFIG`, plutôt que dans `~/.config/nicemailer/config.toml`, que la spécification invite à versionner dans Git.

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

Documentation : `docs/configuration.md`. Écarts au plan : la feature `dev-passwords` (section 2) ; `ConfigError` porte aussi `key` ; `config_status` donne l'état du fichier (règle « un état ne repose jamais sur un événement seul », `evenements.md`) ; schéma en version 4 (`account.disabled_at`, et `account.position`, l'ordre des comptes du sélecteur `@`, modifié par `set_account_order`).

Décision de l'utilisateur à la revue : les mails d'un compte désactivé restent visibles ; seule sa synchronisation s'arrête.


- Crate `nm-engine`, qui ne dépend d'aucune interface (`architecture.md`, principe 1).
- `config.toml` (`configuration.md`) : chemin et `NICEMAILER_CONFIG`, lecture, validation complète (fichier invalide refusé en entier, ligne fautive), clés inconnues signalées, réglages par compte, surveillance FSEvents (crate `notify`) et application sans redémarrage, `nicemailer config check | defaults | path`. Pas encore d'écriture par le contrat : elle arrive avec `add_account` (phase 7).
- Comptes : ligne `account` créée ou réactivée pour chaque clé du fichier ; clé disparue = compte désactivé, données conservées.
- Secrets : interface `Secrets`, dont la seule implémentation de cette phase lit la clé `password` du fichier (compilation de développement seulement). L'hôte et l'identifiant sont enregistrés avec le secret, pour refuser de l'envoyer à un autre serveur.
- SQLite : connexion d'écriture et pool de lecture ; les requêtes du contrat passent par le pool.
- Événements `AccountChanged`, `AccountError`, `ConfigChanged`, `ConfigError` ; `list_accounts`.

Tests : `configuration.md`, section 12, sauf l'écriture ; une lecture n'attend pas une écriture longue.

### Phase 2 : client IMAP (`nm-proto::imap`)

- Connexion TLS directe ou STARTTLS (et `none` réservé à `127.0.0.1`), délais d'attente sur chaque lecture et écriture.
- CAPABILITY (avant et après authentification), LOGIN et AUTHENTICATE PLAIN, ID, ENABLE (CONDSTORE, QRESYNC, UTF8=ACCEPT si annoncés), NAMESPACE, LIST (attributs d'usage spécial si annoncés, délimiteur, noms en UTF-7 modifié), CREATE, STATUS, SELECT et EXAMINE (avec QRESYNC si annoncé), UID FETCH (en-têtes complets, flags, date interne, taille, MODSEQ, corps entier ou par morceaux), UID SEARCH, UID STORE, UID MOVE ou COPY + STORE `\Deleted` + UID EXPUNGE, APPEND avec APPENDUID, IDLE (renouvelé toutes les 25 minutes, RFC 2177), NOOP, LOGOUT.
- Réponses non sollicitées (EXISTS, EXPUNGE, FETCH, VANISHED) remontées à l'appelant, jamais perdues.
- `nicemailer imap probe <compte>` : capacités, dossiers, rôles proposés, sans rien modifier.

Tests : chaque commande contre les deux Dovecot ; transcriptions de réponses réelles rejouées ; coupure au milieu d'un littéral ; réponse malformée (connexion fermée proprement, jamais de plantage).

### Phase 3 : ingestion commune

- Écrivain en flux du stockage brut : morceaux successifs, hash et compression au fil de l'eau, vérification de la taille annoncée, temporaire supprimé en cas d'échec (`stockage-brut.md`, section 5).
- Chemin d'insertion commun à l'import mbox et à IMAP :
  - en-têtes seuls : ligne `message` (`body_state = 0`), adresses, références, threading, entrée plein texte sans corps, compteurs ;
  - message complet : brut, `message_body`, pièces jointes, désinscription, `body_state = 1`, plein texte complet.
- Threading indépendant de l'ordre d'arrivée (réponse avant l'original, fusion de deux conversations), comme pour l'import mbox.

Tests : les tests de l'import mbox restent verts ; un message de 150 Mo en morceaux, mémoire bornée ; disque plein simulé, aucune ligne sans fichier.

### Phase 4 : synchronisation des dossiers (serveur vers base)

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
- **Pannes** : recul exponentiel par compte, plafonné ; reconnexion après une veille du Mac (repérée par un saut de l'horloge) ; `AccountError` en cas d'erreur d'authentification, sans nouvelle tentative avant un changement de configuration (pour ne pas faire verrouiller le compte par le serveur).
- `SyncProgress`, `MessageArrived`, `FolderChanged`, `ThreadChanged`.
- Tant que la phase 5 n'est pas livrée, les commandes qui modifient des mails sont refusées sur les comptes IMAP (sinon la synchronisation suivante les déferait).
- `nicemailer sync status | now | verify`.

Tests : les deux Dovecot ; changements faits par une seconde connexion de test (« le téléphone ») : flags, suppressions, déplacements rapprochés ou non, UIDVALIDITY changé ; coupures à chaque étape ; `sync verify` vert après chaque scénario.

### Phase 5 : spool (base vers serveur)

- Les commandes existantes créent leurs actions dans la même transaction que la modification locale (`modele de donnees.md`, section 6) : `move`, `set_flags`, tags (mots-clés), `delete`.
- Exécuteur sur la connexion de travail : fusion, exécution en série par message, regroupement en un `UID MOVE` par couple de dossiers, COPYUID, rattachement par `Message-ID` sans UIDPLUS ; erreur temporaire (recul) ou définitive (restauration depuis `undo`, actions suivantes annulées, `ActionReverted`) ; message disparu = succès ; reprise des actions `running` au démarrage ; purge.
- Interaction avec la synchronisation : UID d'une action en attente ignoré dans le dossier source, rattachement dans le dossier cible, flags locaux conservés tant que l'action n'est pas exécutée.
- Mots-clés refusés par le serveur (`PERMANENTFLAGS` sans `\*`) : tags et marques de retour restent locaux.
- Valeurs validées par l'utilisateur (5 novembre 2026) pour ce que la spécification laisse ouvert : 8 tentatives au plus, recul de 30 s doublé à chaque échec et plafonné à 1 h, purge des actions terminées après 7 jours.

Tests : `modele de donnees.md`, section 6, scénario par scénario ; deux noyaux qui déplacent le même mail ; plantage simulé entre le MOVE et le passage à `done`.

### Phase 6 : screening à la réception

- Lecture d'`Authentication-Results` (RFC 8601), choix de l'en-tête de confiance, traduction en verdict, alignement par domaine organisationnel (Public Suffix List embarquée, crate `psl` ou liste versionnée comme `public-domains.txt`) ; détection d'`authserv_id` sur les 50 derniers mails de l'Inbox (`authentification-expediteur.md`).
- Algorithme de routage (section 4 du même document), conversation connue, `auth_seen`, `auth_suspect`, ligne `screen` dans `audit_log`, déplacement par le spool, `uid_screening_start`.
- Retour d'une conversation de Todo ou Waiting dans l'Inbox, marques `$NmWasTodo` et `$NmWasWaiting`, retrait de la marque quand un mail marqué quitte l'Inbox ailleurs (`dossiers.md`).
- Mails découverts dans Sent : décision `inbox` pour les destinataires sans décision (`sent_to`), rangement de la conversation selon la présélection pour une réponse envoyée ailleurs.
- Correction de `modele de donnees.md` sur les sous-domaines.

Tests : `authentification-expediteur.md`, section 8 ; `dossiers.md` (retour de Todo/Waiting, mail placé ailleurs, mail de l'utilisateur) ; mail arrivé pendant la synchronisation initiale.

### Phase 7 : création d'un compte et import initial

- Commandes du contrat et CLI : `test_account` (connexion de test), `propose_folder_roles` (attributs d'usage spécial, sinon noms connus en plusieurs langues), `set_folder_roles` (avec création des dossiers manquants), `add_account` (écriture de `config.toml` par `toml_edit`, secret dans le Keychain), `account rename`, `account remove`.
- Keychain (crate `security-framework`), trousseau de session ; le secret de développement en clair reste accepté en compilation de développement.
- Import initial (`import-initial.md`) : comptage, synchronisation des en-têtes de Sent et correspondants, décompte exact par `UID SEARCH`, action `initial_archive` par lots de 1 000 UID, frontière `uid_screening_start`, UIDVALIDITY changé pendant l'import.

Tests : `import-initial.md`, section 11, contre les deux Dovecot ; `configuration.md` pour l'écriture du fichier.

### Phase 8 : robustesse et volume

- Ramasse-miettes des blobs et balayage des orphelins (`stockage-brut.md`, section 7), devenus nécessaires puisque la synchronisation supprime des messages.
- Brut corrompu ou absent : mise à l'écart, puis retéléchargement depuis le serveur.
- Passage du stockage brut à deux niveaux de sous-dossiers (au-delà de 100 000 messages), par une migration.
- Essai de volume : boîte Dovecot générée de 100 000 mails ; durée de la synchronisation initiale, mémoire, réactivité des listes pendant la synchronisation (`make bench`).
- Essai sur le compte Infomaniak réel de l'utilisateur, en lecture d'abord, avec son accord explicite.

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
2. **Fréquence de la comparaison complète des flags** sur un serveur sans CONDSTORE, pour les gros dossiers hors Inbox : à mesurer en phase 4.
3. **Infomaniak et `Authentication-Results`** : quel `authserv-id`, et le serveur retire-t-il les en-têtes entrants qui portent le sien ? À vérifier sur quelques mails reçus.
