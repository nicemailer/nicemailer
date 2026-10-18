# NiceMailer : analyse du schéma SQLite

Ce document justifie le schéma de `schema.sql` (version 1) et reprend les points déjà discutés. Le schéma a été chargé et exercé sur SQLite 3.45.1 : création, recherche plein texte, pagination par curseur, mise à jour et suppression, `integrity_check` et `foreign_key_check` sans erreur.

## 1. Contexte et contraintes

- **Produit** : un client mail rapide, open source, pour Mac.
- **Architecture retenue** : une UI SwiftUI (voir `architecture.md`) cliente d'un noyau Rust, par JSON-RPC sur socket Unix, sans pont FFI. Le noyau doit rester réutilisable (CLI, TUI, web, MCP), donc **il ne contient aucune notion d'UI** et le contrat porte des requêtes et des événements, pas des écrans.
- **Critère dominant** : la fluidité. Une liste de 100 000 messages doit défiler sans image perdue, et l'ouverture d'un mail depuis la base locale doit prendre moins de 100 ms.
- **Conséquence pour la base** : SQLite est la source de vérité locale. L'UI ne lit jamais le réseau, elle lit la base, et la synchronisation IMAP écrit dans la base.

## 2. Principes de conception

| Principe | Application dans le schéma |
|---|---|
| **Table de liste étroite** | `message` ne contient que des en-têtes et des champs dénormalisés (expéditeur, aperçu). Le corps vit dans `message_body`. |
| **Pas d'`OFFSET`** | Pagination par curseur `(date_received, id)`, avec l'index `idx_message_list`. Le coût reste constant à toute profondeur de défilement. |
| **Index partiels pour les filtres fréquents** | `idx_message_unread` et `idx_message_flagged` ne contiennent que les lignes concernées. |
| **Corps séparés de la table de liste** | Les corps (`message_body`), volumineux, vivent à part pour que la liste reste légère. |
| **Hors-ligne d'abord** | Toute action est appliquée localement tout de suite et enregistrée dans le spool `pending_action`, qui la rejoue vers le serveur et l'annule en cas d'échec définitif (section 6). |
| **Aucun secret en base** | `account.keychain_ref` référence le Keychain, jamais le mot de passe ou le jeton. |
| **Typage strict** | Tables `STRICT`, contraintes `CHECK` sur les booléens et les énumérations, `WITHOUT ROWID` sur les tables de liaison. |

## 3. Les tables et leur justification

### `account`
Paramètres IMAP/SMTP et mode d'authentification (`password` ou `oauth2`). Les secrets sont dans le Keychain. Chaque compte a ses propres dossiers, messages et conversations (suppression en cascade).

### `folder`
Un dossier IMAP, identifié par son chemin complet. Les colonnes `uidvalidity`, `uidnext` et `highestmodseq` pilotent la synchronisation incrémentale (voir section 5). `role` associe un dossier serveur à l'un des dossiers du workflow défini dans `dossiers.md` (inbox, todo, waiting, archive, sent, newsletter, paperwork, quarantine, deleted, junk, banned, drafts), quelle que soit sa langue ou son nom côté serveur. Les dossiers sans rôle ne sont pas gérés. `total_count` et `unread_count` sont des compteurs dénormalisés, pour afficher la barre latérale sans `COUNT(*)`.

### `message`
Le cœur du modèle. Une ligne par copie d'un message sur le serveur.
- **Identité** : `id` est l'identité stable du message. Elle ne change pas quand le message est déplacé : un déplacement est un `UPDATE` de `folder_id` et `uid`, jamais une suppression suivie d'une insertion. Le corps, les pièces jointes, les mots-clés, les références, les destinataires et l'entrée plein texte restent donc attachés au message.
- **`uid`** : l'adresse du message sur le serveur. Le contrat ne l'expose jamais, mais le moteur en a besoin pour envoyer les ordres (`UID MOVE`, `UID STORE`) et pour interpréter la synchro (flags modifiés, `VANISHED`, `uidnext`), qui désigne les messages par UID. D'où la contrainte `UNIQUE (folder_id, uid)`. `uid` vaut NULL pendant un déplacement, tant que le serveur n'a pas attribué l'UID dans le dossier cible (section 6) ; SQLite autorise plusieurs NULL dans une contrainte UNIQUE.
- **Dénormalisation volontaire** de `from_name`, `from_addr` et `snippet` : la liste n'a jamais de jointure à faire pour dessiner une ligne.
- **`date_received`** (INTERNALDATE) sert de clé de tri, car l'en-tête `Date` est librement falsifiable ou absent. `date_sent` est conservé pour l'affichage.
- **Flags** en colonnes séparées (`is_seen`, `is_flagged`...), plus rapides à filtrer et à indexer qu'une chaîne de flags. Les mots-clés personnalisés vont dans `message_keyword`.
- **`modseq`** permet de détecter les changements de flags (CONDSTORE).
- **`body_state`** distingue les états 0 (en-têtes seuls) et 1 (message complet téléchargé), pour savoir s'il faut récupérer le message à l'ouverture.

### `message_body`
Texte brut et HTML déjà assaini (ammonia), plus un indicateur `has_remote_content` pour proposer « charger les images distantes ». C'est une **interprétation** du message brut : seul le brut enregistré sur disque est immuable (voir `stockage-brut.md`). Le corps est écrit à la réception du message complet, puis réécrit par `reindex` quand l'analyseur MIME ou l'assainissement évoluent : correction d'une faille dans l'assainissement, nouvelle détection des pixels de suivi, meilleur décodage. `parser_version` indique avec quelle version il a été produit, et `reindex` cible les lignes dont la version est inférieure à la version courante. Ce n'est pas un cache : il n'est jamais évincé, et seule la suppression du message l'efface. `raw_path` pointe vers le `.eml` brut s'il est conservé sur disque.

### `thread` et `thread_folder`
Les conversations (algorithme JWZ). Une conversation peut couvrir plusieurs dossiers (par exemple Reçus et Envoyés), d'où `thread_folder`, qui est une **vue matérialisée par dossier** : la liste de conversations d'un dossier devient un simple parcours d'index (`idx_thread_folder_list`), sans agrégation à chaque défilement.

### `message_reference`
L'en-tête `References`, éclaté une ligne par référence et ordonné par `position`, avec un index sur `ref_msgid`. C'est la matière première du threading : retrouver tous les messages qui citent un identifiant donné.

### `message_address`
Destinataires (`to`, `cc`, `bcc`, `reply_to`), dans une table séparée parce qu'ils sont en nombre variable et rarement affichés dans la liste. Chaque ligne référence la table `address` par `address_id` ; le nom d'affichage n'y figure plus, il est dans `address.name`. L'index sur `address_id` retrouve tous les messages liés à un contact. L'autocomplétion s'appuie sur `address`.

### `message_keyword`
Mots-clés IMAP d'un message, avec un index dans les deux sens (message vers mots-clés, mot-clé vers messages). Les tags de l'utilisateur y sont stockés, mais tous les mots-clés ne sont pas des tags.

### `tag`
Les tags de `dossiers.md` (classement fin, plusieurs par mail) correspondent aux **mots-clés IMAP** du serveur : c'est l'équivalent standard, un message pouvant en porter plusieurs. `tag` fait le lien : `name` est le nom affiché, `keyword` le mot-clé envoyé au serveur. Un tag s'applique à un message quand `message_keyword.keyword = tag.keyword`. Les mots-clés sans ligne dans `tag` (`$Forwarded`, `$MDNSent`…) sont techniques et ne sont pas affichés.

Limites de cette équivalence :
- le serveur doit accepter les mots-clés personnalisés (`PERMANENTFLAGS` contenant `\*`) ; sinon les tags restent locaux et ne se synchronisent pas ;
- un mot-clé est un atome ASCII sans espace ni accent : le nom affiché est encodé, et la règle d'encodage reste à définir ;
- tous les clients ne les affichent pas (Thunderbird oui, Mail sur iOS en général non) ; Gmail ne les expose pas comme libellés. Le téléphone ne montrera donc pas toujours les tags ;
- un mot-clé est porté par chaque copie d'un message (par dossier et UID) : un déplacement doit le conserver.

### `address` et `domain`
Table des contacts : adresses et domaines connus de l'application. Adresses et domaines existent indépendamment des comptes (pas de `account_id`, `addr` et `domain` sont uniques). Leur premier attribut est la décision de screening (`screening_target`, `screening_decided_at`), d'autres attributs pourront s'ajouter ; sans décision, ces colonnes sont NULL. `address` porte aussi le nom d'affichage du contact (`name`) : un seul par adresse, ce qui compacte la base et permet à l'utilisateur de le corriger une fois pour tous les messages. Il est renseigné à la première rencontre de l'adresse et n'est jamais écrasé ensuite par l'ingestion. `address` associe une adresse à un dossier cible, `domain` un domaine. Les adresses des destinataires (`message_address`) pointent vers `address`. Cible possible : `inbox`, `newsletter`, `paperwork` ou `banned`, jamais `quarantine`. À l'arrivée d'un mail, la règle d'adresse l'emporte sur la règle de domaine, et sans règle le mail va en Quarantine. Déplacer un mail de Quarantine vers un dossier crée ou met à jour la règle d'adresse. La correspondance de domaine est exacte (pas de sous-domaine) dans cette version.

### `attachment`
Métadonnées seulement. Les fichiers vivent sur disque et `disk_path` reste `NULL` tant que la pièce jointe n'est pas téléchargée. `part_id` est le numéro de partie MIME, `content_id` sert aux images inline (`cid:`).

### `message_fts`
Index plein texte FTS5 sur objet, expéditeur, destinataires et corps, avec `unicode61 remove_diacritics 2` (« eleve » trouve « élève ») et des index de préfixes 2 et 3 pour la saisie progressive. Des triggers le maintiennent à jour (section 7).

### `pending_action`
Le spool : toutes les instructions envoyées au serveur passent par cette table (marquer lu, déplacer, copier, supprimer, enregistrer un brouillon, envoyer). Chaque ligne porte les paramètres de l'ordre (`payload`), l'état local à restaurer en cas d'échec définitif (`undo`), l'état d'exécution, le nombre de tentatives, la dernière erreur et `run_after` pour le recul exponentiel. L'index `idx_pending_action_queue` sert la requête « prochaine action à exécuter », et `idx_pending_action_message` retrouve les actions en attente sur un message. Le fonctionnement est décrit en section 6.

## 4. Pagination par curseur : pourquoi pas `OFFSET`

Avec `OFFSET`, SQLite doit lire et écarter toutes les lignes précédentes, donc le défilement ralentit à mesure qu'on descend. Le curseur `(date_received, id)` est une comparaison de ligne (`(date_received, id) < (:d, :id)`) que l'index `idx_message_list` résout directement. Le plan d'exécution vérifié est :

```
SEARCH message USING INDEX idx_message_list (folder_id=? AND (date_received,rowid)<(?,?))
```

L'index est déclaré sur `(folder_id, date_received)` seulement : SQLite ajoute toujours le `rowid` (ici `id`) à la fin d'un index, ce qui fournit le tiebreaker et garantit un ordre total, même si plusieurs messages partagent la même seconde. L'index est déclaré croissant et parcouru à l'envers pour `ORDER BY date_received DESC, id DESC`. Le déclarer `date_received DESC` casserait cette correspondance, car le `rowid` implicite reste croissant, et SQLite ajouterait un tri. Plan vérifié sur SQLite 3.54 (20 000 lignes) : aucun tri supplémentaire.

L'index ne contient pas les colonnes affichées (`subject`, `snippet`…) : chaque ligne de la page est lue dans la table, ce qui est négligeable pour 50 lignes.

## 6. Actions locales et spool

Toutes les instructions envoyées au serveur passent par le spool `pending_action`. L'utilisateur voit le résultat d'une action immédiatement ; si le serveur la refuse définitivement, l'état local est restauré.

### Principe

La table `message` est modifiée directement, et l'action garde de quoi l'annuler (`undo`). L'alternative, qui laisserait la base fidèle au serveur et superposerait les actions en attente à chaque lecture, est écartée : elle empêcherait d'utiliser les index et la pagination par curseur.

### Déplacement d'un message

1. **Commande utilisateur**, dans une seule transaction :
   - `UPDATE message SET folder_id = <cible>, uid = NULL` ;
   - mise à jour des compteurs dénormalisés (`folder`, `thread_folder`) ;
   - insertion d'une action `move`, avec `payload = {from_folder_id, from_uid, to_folder_id}` et `undo = {folder_id, uid}` ;
   - émission de l'événement de changement.
2. **Exécution par le spool** : `UID MOVE` (RFC 6851). Sans l'extension MOVE, la même action exécute `UID COPY`, `UID STORE +FLAGS (\Deleted)` puis `UID EXPUNGE` : c'est toujours un seul ordre logique du spool.
3. **Succès** : le nouvel UID est donné par la réponse `COPYUID` (UIDPLUS) et enregistré par `UPDATE message SET uid = …`. Sans UIDPLUS, `uid` reste NULL et la synchro du dossier cible rattache le message (voir « Synchro et spool »).

Les autres actions (`set_flags`, `delete`…) suivent le même schéma : modification locale et action dans la même transaction, `undo` contenant les valeurs précédentes.

### Erreurs temporaires et définitives

- **Temporaire** (réseau absent, délai dépassé, serveur indisponible) : l'action reste `queued`, `attempts` est incrémenté et `run_after` repoussé (recul exponentiel). Pas d'annulation : sinon le mode hors ligne serait inutilisable.
- **Définitive** (réponse `NO` du serveur, dossier cible inexistant, quota dépassé, nombre maximal de tentatives atteint) : l'état local est restauré depuis `undo`, les compteurs sont corrigés dans la même transaction, l'action passe à `reverted` et l'événement `ActionReverted {message_id, kind, reason}` est émis. L'interface affiche une notification non bloquante ; pour les déplacements automatiques du screening, l'annulation est seulement journalisée.
- **Message disparu du serveur** (supprimé ailleurs, par exemple depuis le téléphone) : restaurer l'état n'a pas de sens, le message est supprimé localement.

### Actions successives sur un même message

- **Fusion** : si une action `move` est encore `queued` quand un nouveau déplacement du même message est demandé, les deux sont fusionnées. Par exemple, Inbox → Todo puis Todo → Archive devient Inbox → Archive. L'action fusionnée garde le `undo` de la première ; la seconde n'est pas créée.
- **Exécution en série** : les actions d'un même message s'exécutent dans l'ordre de leur `id`, une à la fois.
- **Annulation en série** : un échec définitif annule l'action fautive et toutes les suivantes sur ce message (`cancelled`), puis restaure l'état antérieur à l'action fautive.

### Synchro et spool

La synchro consulte le spool (index `idx_pending_action_message`) pour ne pas défaire une action locale :

- **Dossier source** : tant qu'un `move` est en attente, le serveur montre encore `from_uid` dans le dossier source. La synchro ignore tout UID référencé par une action `queued` ou `running` : elle ne le réinsère pas comme un nouveau message.
- **Dossier cible** : un UID inconnu est d'abord rapproché des lignes du même dossier dont `uid` est NULL et dont le `message_id_hdr` est identique ; s'il y a correspondance, `uid` est renseigné au lieu d'insérer un doublon.
- **Flags** : un changement de flags venant du serveur sur un message qui a une action `set_flags` en attente ne remplace pas la valeur locale avant l'exécution de l'action.

### Reprise après plantage

Au démarrage, les actions `running` repassent à `queued`. Chaque action doit pouvoir être rejouée sans effet de bord : un `MOVE` peut avoir réussi sans que l'action soit passée à `done`. Si `from_uid` n'existe plus dans le dossier source, le moteur cherche le message dans le dossier cible (`UID SEARCH HEADER Message-ID`) avant de conclure à un échec. S'il l'y trouve, l'action est `done` et `uid` est renseigné.

### Purge

Les actions `done`, `reverted` et `cancelled` sont supprimées après un délai (paramètre à fixer), pour que la table reste petite.

## 5. Synchronisation IMAP : `uidvalidity`, `uidnext`, `highestmodseq`

Ces trois valeurs, stockées par dossier, permettent de ne retélécharger que ce qui a changé.

- **`uidvalidity`** : numéro d'époque des UID. S'il diffère de la valeur stockée, les UID ont été réattribués (restauration, migration, dossier recréé) et ne désignent plus les mêmes messages. Il faut alors **purger le dossier** (corps et pièces jointes compris) et le resynchroniser.
- **`uidnext`** : l'UID du prochain message à arriver. S'il est supérieur à la valeur stockée, les nouveaux messages sont ceux dont l'UID est supérieur ou égal à l'ancienne valeur (`UID FETCH <ancien_uidnext>:*`).
- **`highestmodseq`** : le plus grand numéro de modification du dossier (extension CONDSTORE). Il détecte les changements de flags et, avec QRESYNC, les suppressions (`VANISHED`). Sans CONDSTORE côté serveur, la colonne reste `NULL` et le client retombe sur une comparaison complète des flags, plus lente.

Cycle de synchronisation à l'ouverture d'un dossier :

1. `uidvalidity` différent : tout purger, resynchroniser le dossier.
2. `highestmodseq` supérieur : récupérer les flags modifiés et les suppressions.
3. `uidnext` supérieur : récupérer les en-têtes des nouveaux UID.
4. Enregistrer les nouvelles valeurs **dans la même transaction SQLite** que les messages. Après un plantage, on reprend ainsi à un état cohérent, sans message manquant ni doublon.

## 7. Recherche plein texte

- **Insertion** : le trigger `message_fts_ai` indexe l'objet et l'expéditeur dès la synchronisation des en-têtes, avec un corps vide. La recherche fonctionne donc avant tout téléchargement de corps.
- **Arrivée du corps** : les triggers de `message_body` ajoutent le texte à l'index.
- **Mise à jour et suppression** : `message_fts_au` suit les changements d'objet ou d'expéditeur, `message_fts_ad` retire la ligne à la suppression du message.
- **Réinterprétation du corps** : quand `reindex` réécrit `message_body.text_plain`, le trigger `message_body_fts_au` met l'index à jour. `reindex` procède par `UPDATE` : un `INSERT OR REPLACE` supprimerait puis réinsérerait la ligne sans déclencher ce trigger.
- **Coût** : la table FTS5 est autonome (elle stocke sa propre copie du texte). C'est le choix le plus simple, au prix d'un espace disque supplémentaire. Une table à contenu externe, ou sans contenu (`contentless_delete`, SQLite 3.43 ou plus, mais sans `snippet()`), économiserait cet espace ; à évaluer selon la taille réelle des bases.

## 8. Remarques déjà traitées

| Remarque | Réponse |
|---|---|
| **GPUI ou Swift ?** | Swift pour l'UI et un noyau Rust : le rendu HTML (`WKWebView`), l'édition de texte riche (`NSTextView`, intégrable à SwiftUI), l'accessibilité et l'intégration macOS existent déjà et sont de qualité. Le gain de fluidité de GPUI est marginal face aux listes natives à cellules recyclées. GPUI ne se justifierait que par une exigence multiplateforme ou « tout Rust ». |
| **Où passe la frontière Rust / Swift ?** | Le Rust gère protocoles, parsing, stockage, synchronisation et threading. Le Swift ne contient aucune logique métier. Le contrat JSON-RPC expose des requêtes (« 50 messages à partir de ce curseur ») et des événements de changement. |
| **Comment valider la fluidité ?** | Prototype sur 100 000 messages synthétiques mesurés avec Instruments : aucune image perdue au défilement, ouverture d'un mail depuis la base locale en moins de 100 ms, nouveau mail visible en quelques secondes via IDLE. |
| **À quoi servent `uidvalidity`, `uidnext`, `highestmodseq` ?** | Voir section 5. |
| **La recherche trouve-t-elle les accents ?** | Oui, grâce à `remove_diacritics 2`. Le test « eleve » retrouve « Élève facture ». |
| **La liste est-elle rapide à grande profondeur ?** | Oui, grâce à la pagination par curseur et à l'index `idx_message_list`. Plan vérifié en section 4. |
| **Peut-on libérer de la place sans perdre de mails ?** | Non : le brut est immuable et conservé intégralement, et le corps en est une interprétation qui n'est jamais évincée. On libère de la place en supprimant des messages (le ramasse-miettes retire alors les blobs sans référence). |

## 9. Limites connues et points d'attention

- **Threading non automatique** : `thread` et `thread_folder` sont maintenues par le code Rust. Aucun trigger ne calcule les conversations. C'est à implémenter tôt, car cela conditionne le modèle de la liste.
- **Colonne `recipients` de l'index plein texte** : elle reste vide, à remplir depuis `message_address` quand les destinataires sont connus.
- **Pragmas par connexion** : `foreign_keys`, `synchronous` et `temp_store` doivent être réappliqués à chaque ouverture de connexion. Sans `foreign_keys = ON`, les suppressions en cascade ne s'exécutent pas. Seul `journal_mode = WAL` persiste dans le fichier.
- **Compteurs dénormalisés** (`folder.total_count`, `unread_count`, `thread.*`, `thread_folder.*`) : ils doivent être mis à jour dans la même transaction que les messages, sous peine de dériver.
- **Même message dans plusieurs dossiers** : une ligne par copie sur le serveur. Chez Gmail, un message portant plusieurs étiquettes apparaît plusieurs fois. L'index sur `message_id_hdr` permet de les dédoublonner à l'affichage.
- **Migrations** : `user_version` vaut 1. Au démarrage, le code compare cette valeur à celle qu'il attend et applique les migrations manquantes dans l'ordre.
- **Version de SQLite** : le schéma exige SQLite 3.38 ou plus (tables `STRICT`, `unixepoch()`) avec FTS5. Avec `rusqlite` et la fonctionnalité `bundled`, la version embarquée est maîtrisée et ne dépend pas de celle du système.
- **OAuth Gmail** : sans effet sur le schéma, mais les scopes mail sont « restreints » et exigent une vérification Google avant diffusion publique. Commencer par IMAP et mot de passe d'application.

## 10. Suite proposée

1. Couche Rust (`rusqlite`) : ouverture de connexion avec les pragmas, migrations par `user_version`, requêtes de liste par curseur.
2. Contrat `nm-api` de la liste de messages (page, curseur, événements de changement).
3. Implémentation du threading JWZ et de la maintenance de `thread_folder`.
