# NiceMailer : traçabilité

Statut : validé. Complète `modele de donnees.md` et `schema.sql`.

## 1. Objet

Le screening, la synchronisation entre Mac et l'agent MCP agissent sur la boîte sans que l'utilisateur soit toujours devant l'écran. Il doit pouvoir répondre à trois questions :

- **Qui a fait quoi, et quand ?** Qui a banni cette adresse, qui a déplacé ce mail, qu'a fait l'agent dans ma boîte ?
- **Qu'ai-je traité aujourd'hui ?** Les derniers mails déplacés, tagués ou marqués, pour faire le bilan de sa journée.
- **Qu'ai-je consulté récemment ?** Pendant une recherche longue dans l'historique : « je viens de voir un autre mail sur le même sujet ».

Trois mécanismes y répondent :

1. des **colonnes de traçabilité** sur les entités, pour le tri et l'affichage ;
2. une **date de dernière consultation** sur les messages ;
3. un **journal** (`audit_log`) en ajout seul, pour l'historique complet.

## 2. Colonnes de traçabilité

### Les quatre colonnes

| Colonne | Contenu |
| --- | --- |
| `date_created` | Création de la ligne |
| `date_modified` | Dernière modification d'un attribut visible de l'utilisateur |
| `created_by` | Source de la création |
| `modified_by` | Source de la dernière modification |

À la création, `date_modified = date_created` et `modified_by = created_by`.

### Périmètre

| Table | Colonnes de traçabilité | Raison |
| --- | --- | --- |
| `account`, `folder`, `address`, `domain`, `tag`, `message` | Les quatre | Entités que l'utilisateur crée et modifie |
| `pending_action` | `date_created`, `date_modified` (changement d'état), `created_by` (client qui a donné l'ordre) | Journal technique du spool ; la modification vient toujours du noyau |
| `audit_log` | `date_created`, `created_by` | Lignes jamais modifiées |
| `message_reference`, `message_address`, `message_keyword` | Aucune | Lignes de liaison créées avec le message ; ajouter un tag modifie le **message** |
| `thread`, `thread_folder` | Aucune | Agrégats recalculés en permanence |
| `message_body` | `date_fetched` seulement | Un `reindex` réécrit le corps sans que rien ne change pour l'utilisateur |
| `message_fts` | Aucune | Table virtuelle FTS5 |

### Ce qui compte comme une modification

`date_modified` ne suit que les attributs visibles de l'utilisateur. Les colonnes techniques, que la synchronisation met à jour sans cesse, n'y touchent pas : sinon la vue « derniers mails traités » serait inutilisable.

| Table | Modifie `date_modified` | Ne le modifie pas |
| --- | --- | --- |
| `message` | Dossier, `is_seen`, `is_flagged`, `is_answered`, `is_draft`, `is_deleted`, tags (`message_keyword`) | `uid`, `modseq`, `body_state`, `snippet`, `date_last_viewed` |
| `address` | `name`, décision de screening, `auth_policy` | `auth_seen` |
| `domain` | Décision de screening | |
| `folder` | Rôle, abonnement | Compteurs, `uidvalidity`, `uidnext`, `highestmodseq`, `date_last_sync` |
| `tag` | Nom, couleur, suppression | |
| `account` | Renommage de la clé (`config_key`) | `authserv_id` détecté automatiquement ; les paramètres du compte sont dans `config.toml` (`configuration.md`) |

C'est le code Rust qui renseigne la date et la source, dans la même requête que la modification. Un trigger ne conviendrait pas : il ne connaît pas la source et se déclencherait aussi sur les colonnes techniques.

Les dates par attribut de `address`, `domain` et `folder` (`date_screening_modified`, `date_auth_policy_modified`, `date_role_modified`) restent nécessaires : la synchronisation de la configuration fusionne attribut par attribut (`synchronisation-configuration.md`).

## 3. Sources

| Valeur | Source |
| --- | --- |
| `core` | Le noyau : réception d'un mail, screening automatique, détection automatique |
| `app` | L'application SwiftUI |
| `cli` | Le CLI |
| `tui` | La TUI |
| `web` | Le serveur web |
| `mcp` | L'agent, par le proxy MCP |
| `config_sync` | La synchronisation de la configuration entre Mac |
| `imap` | Un changement vu sur le serveur : téléphone, webmail, autre client, autre Mac |

**La source n'est jamais déclarée par le client.** Le noyau la déduit de l'identité vérifiée du client (signature de code, `architecture.md`, section 9). Un client ne peut donc pas se faire passer pour un autre, et la traçabilité est fiable.

Création d'un message :

- mail reçu dans l'Inbox et trié : `core` ;
- message apparu dans un autre dossier, sans action locale correspondante (envoyé depuis le téléphone, déplacé ailleurs) : `imap` ;
- brouillon ou mail envoyé depuis NiceMailer : le client qui l'a rédigé.

Pour une modification reçue par `config_sync`, les dates prennent l'horodatage de l'entrée reçue, pas l'heure locale (`synchronisation-configuration.md`, section 10).

## 4. Dernière consultation : `date_last_viewed`

`message.date_last_viewed` est la date de la dernière consultation du message par un humain.

- **Commande explicite.** Le noyau ne sait pas quand un mail est réellement affiché, puisqu'un client peut précharger des corps. Le client envoie `mark_viewed(message_id)`.
- **Seuil de 2 s.** Le client n'envoie la commande que si le message est resté affiché au moins `ui.view_threshold_ms` (2 000 ms). Naviguer au clavier dans une liste ne marque donc pas tous les mails survolés.
- **Clients humains seulement** : `app`, `tui` et `web`. Le noyau refuse `mark_viewed` venant du CLI ou de l'agent MCP (droits par client, `architecture.md`, section 9).
- **Ce n'est pas une modification** : `date_modified` et `modified_by` ne changent pas, et rien n'est inscrit dans `audit_log`.
- **Locale** : la date n'est envoyée ni au serveur ni aux autres Mac.
- Un événement `MessageChanged` est émis, pour que la vue « derniers consultés » se mette à jour.

Le nom évite `seen`, qui désigne le flag IMAP `\Seen` (lu, `is_seen`).

## 5. Journal `audit_log`

### Rôle

Les colonnes ne gardent que la **dernière** modification. Une adresse bannie par moi en mars, réautorisée par un collègue en juin puis rebannie par la synchronisation n'affiche que « `config_sync`, juin ». Le journal garde l'historique complet.

```sql
CREATE TABLE audit_log (
    id           INTEGER PRIMARY KEY,
    date_created INTEGER NOT NULL DEFAULT (unixepoch()),
    created_by   TEXT NOT NULL,           -- source (section 3)
    entity       TEXT NOT NULL,           -- account, folder, address, domain, tag, message
    entity_id    INTEGER NOT NULL,
    action       TEXT NOT NULL,
    detail       TEXT                     -- JSON : {"before": …, "after": …, …}
) STRICT;
```

- **Ajout seul** : une ligne n'est jamais modifiée.
- **Pas de clé étrangère** : l'historique survit à la suppression de l'entité. `detail` contient de quoi l'afficher sans elle (adresse, objet du mail, nom du dossier).
- Inscrit **dans la même transaction** que la modification.

### Ce qui est journalisé

| Entité | Actions | `detail` |
| --- | --- | --- |
| `address`, `domain` | `screening`, `auth_policy`, `rename` | Avant, après ; pour `screening`, le message qui a déclenché la décision (sortie de Quarantine) |
| `folder` | `role` | Avant, après |
| `tag` | `create`, `update`, `delete` | Avant, après |
| `account` | `create`, `rename`, `disable`, `remove` | Clé du compte ; pour `disable`, compte absent de `config.toml` ; jamais de secret |
| `config` | `update` | Modification par une commande du contrat : clé, avant, après (`configuration.md`, section 10) |
| `config` | `reload` | Édition directe de `config.toml` constatée au rechargement, source `core` : clés changées |
| `account` | `initial_archive` | Critères et nombre de mails archivés à l'import initial (`import-initial.md`) |
| `message` | `screen` | Dossier cible, règle appliquée (adresse, domaine, conversation connue, aucune), verdict d'authentification |
| `message` | `move`, `set_flags`, `set_tags`, `delete`, `send` | Avant, après ; pour un rangement déclenché par une décision, l'adresse ou le domaine concerné ; pour un retour de Todo ou de Waiting dans l'Inbox, le mail reçu qui l'a déclenché ; pour un rangement dans Archive après une réponse, le mail envoyé |
| `message` | `revert` | Action annulée après un échec définitif du serveur, et sa raison |

### Ce qui n'est pas journalisé

- Les changements de source `imap` : les flags et déplacements faits ailleurs sont nombreux, sans auteur identifiable, et la boîte du serveur en garde déjà l'état.
- `mark_viewed` : `date_last_viewed` suffit.
- Les mises à jour techniques (`uid`, `modseq`, compteurs, `reindex`).

### Volume et purge

Le volume est dominé par `screen`, une ligne par mail reçu, de l'ordre de 150 octets : quelques Mo par an pour un usage courant. Les lignes plus anciennes que `audit.retention_days` (365 jours) sont purgées.

## 6. Interface et contrat

### Vues de messages

La vue par défaut reste la liste de conversations d'un dossier, triée par `date_received`. Deux vues s'y ajoutent ; ce sont des listes de **messages**, tous dossiers confondus :

| Vue | Tri | Filtre par défaut | Index |
| --- | --- | --- | --- |
| Derniers mails traités | `date_modified` décroissant | `modified_by` parmi `app`, `cli`, `tui`, `web`, `mcp` | `idx_message_modified` |
| Derniers mails consultés | `date_last_viewed` décroissant | `date_last_viewed` renseignée | `idx_message_viewed` (partiel) |

Le filtre de la première vue est modifiable : ajouter `core` pour voir aussi le screening automatique, ou ne garder que `mcp` pour voir ce qu'a fait l'agent. La pagination par curseur fonctionne comme pour `date_received`, avec le couple (date, `id`).

### Historique

- La fiche d'une adresse ou d'un domaine affiche son historique : qui l'a classée, quand, depuis quel message.
- Un message affiche son historique : comment il a été trié, qui l'a déplacé ou tagué.

### Contrat

| Nom | Type | Rôle |
| --- | --- | --- |
| `list_messages(order, sources?, cursor, limit)` | requête | `order` : `received`, `modified` ou `viewed` ; `sources` filtre `modified_by` |
| `mark_viewed(message_id)` | commande | Met à jour `date_last_viewed` ; app, TUI et web seulement |
| `get_history(entity, entity_id, cursor, limit)` | requête | Historique d'une entité tiré de `audit_log` |
| `list_history(since?, sources?, cursor, limit)` | requête | Journal global, par exemple « tout ce qu'a fait l'agent aujourd'hui » |

## 7. Limites

- **Boîte partagée.** `config_sync` dit que la modification vient d'un autre Mac, pas de quelle personne. Identifier l'auteur demanderait d'ajouter un nom de poste aux entrées du document de configuration ; c'est à reconsidérer à l'usage.
- **Déplacements faits ailleurs.** Un mail déplacé depuis le téléphone est vu par la synchronisation comme une disparition dans un dossier et une apparition dans un autre. Il n'apparaît donc pas dans « derniers mails traités ».
- **Horloges.** Les dates sont celles du Mac. Un Mac mal réglé produit des dates fausses, que la synchronisation propage.

## 8. Paramètres

Ces paramètres sont des clés de `config.toml` ; le catalogue complet est dans `configuration.md`, section 11.

| Paramètre | Rôle | Valeur initiale |
| --- | --- | --- |
| `ui.view_threshold_ms` | Durée d'affichage avant `mark_viewed` | 2 000 |
| `audit.retention_days` | Durée de conservation du journal | 365 |

## 9. Tests exigés

- Mise à jour d'une colonne technique (`uid`, `modseq`, `snippet`, compteurs) : `date_modified` et `modified_by` inchangés.
- Déplacement par l'app, par le CLI, par l'agent : `modified_by` vaut respectivement `app`, `cli`, `mcp`, quel que soit ce que le client prétend être.
- Screening d'un mail reçu : `created_by = 'core'` et une ligne `screen` dans `audit_log`.
- Changement de flag venu du serveur : `modified_by = 'imap'`, aucune ligne dans `audit_log`.
- Modification reçue par `config_sync` : dates égales à l'horodatage reçu, ligne dans `audit_log`.
- `mark_viewed` : `date_last_viewed` mise à jour, `date_modified` inchangée ; refusé depuis le CLI et le MCP.
- Suppression d'une adresse ou d'un message : son historique reste lisible.
- Purge : lignes plus anciennes que `audit.retention_days` supprimées, les autres conservées.
- Vues « traités » et « consultés » : pagination par curseur sans trou ni doublon.
