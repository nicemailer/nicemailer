# NiceMailer : configuration

Statut : validé pour la v1. Complète `architecture.md`, `schema.sql`, `modele de donnees.md` et `synchronisation-configuration.md`.

## 1. Objet

Ce document spécifie où et comment sont stockés les réglages de NiceMailer et les paramètres des comptes.

## 2. Principes

- **La configuration est un fichier texte TOML.** L'app offre une interface pour que l'utilisateur novice n'ait jamais à l'ouvrir. Le fichier texte permet à l'utilisateur avancé de le versionner dans Git et de le partager entre ses Mac, et à un agent de le modifier simplement.
- **Le fichier contient les intentions de l'utilisateur, la base contient l'état.** Ce que l'utilisateur choisit (un serveur, un intervalle, une identité) est dans le fichier. Ce qui est détecté (`authserv_id`), calculé ou décidé au fil de l'eau (screening, tags) est en base.
- **Le noyau seul lit et écrit le fichier.** Les clients passent par le contrat ; l'utilisateur et les agents peuvent aussi l'éditer directement.
- **Aucun secret dans le fichier.** Les mots de passe et les jetons sont dans le Keychain.

## 3. Emplacement et forme

Le fichier est `~/.config/nicemailer/config.toml`. Ce chemin est celui des outils en ligne de commande et des dépôts de dotfiles ; `~/Library/Application Support`, où vit la base, n'est pas un endroit où l'on pense à chercher un fichier à versionner. La variable d'environnement `NICEMAILER_CONFIG` permet d'en désigner un autre.

- **Le fichier ne contient que ce qui diffère des valeurs par défaut.** Une clé absente prend sa valeur par défaut. Un fichier absent équivaut à un fichier vide : NiceMailer démarre sans compte.
- **Les noms de paramètres des spécifications sont des clés TOML.** `sync.poll_interval_secs` est la clé `poll_interval_secs` de la table `[sync]`.
- **Réglages par compte.** Un compte est une table `[accounts.<clé>]`. Un réglage général peut y être redéfini pour ce compte (`[accounts.perso.sync]`) ; la valeur du compte l'emporte.
- **Version du format.** La clé `version` (1) permet de migrer le fichier si son format change.

Exemple :

```toml
version = 1

[core]
background_sorting = true

[sync]
poll_interval_secs = 300

[accounts.perso]
email = "prenom@exemple.fr"
display_name = "Prénom Nom"
auth = "password"            # ou "oauth2"
config_sync = false          # synchronisation-configuration.md

[accounts.perso.imap]
host = "imap.exemple.fr"
port = 993
security = "tls"             # tls, starttls ou none
username = "prenom"          # par défaut : email

[accounts.perso.smtp]
host = "smtp.exemple.fr"
port = 465
security = "tls"
username = "prenom"

[[accounts.perso.identities]]
email = "contact@exemple.fr"
name = "Prénom Nom"
```

## 4. Lecture et application

- **Au démarrage**, le noyau lit le fichier et le valide.
- **En cours de route**, il le surveille (FSEvents) et applique les changements sans redémarrage.
- **Fichier invalide** (syntaxe, type, valeur hors limites) : il est refusé en entier. Le noyau garde la configuration précédente, émet l'événement `ConfigError {line, message}` et l'app affiche un bandeau qui renvoie à la ligne fautive. Au démarrage, sans configuration précédente, le noyau démarre avec les valeurs par défaut et sans compte, et signale l'erreur de la même façon. Appliquer une partie seulement d'un fichier invalide produirait un état que personne n'a écrit.
- **Clé inconnue** : avertissement, pas erreur. Un fichier écrit par une version plus récente de NiceMailer reste ainsi lisible par une plus ancienne.

## 5. Écriture par l'interface

Les réglages modifiés dans l'app, la TUI ou le CLI le sont par des commandes du contrat (`set_config`, `add_account`…). Le noyau écrit alors le fichier :

- **en préservant commentaires, ordre et mise en forme** (bibliothèque `toml_edit`) : un fichier versionné par l'utilisateur ne doit pas être réécrit de fond en comble pour une clé ;
- **de façon atomique** : fichier temporaire, puis `rename` ;
- **sans écraser une édition manuelle** : si le fichier a changé sur le disque depuis sa dernière lecture, le noyau le relit d'abord, puis n'applique que la clé modifiée ;
- **sans écrire de valeur par défaut** : remettre un réglage à sa valeur par défaut supprime la clé.

Le noyau n'écrit jamais le fichier de sa propre initiative, seulement à la demande d'un client. En particulier, la synchronisation entre Mac ne le modifie pas (section 8).

## 6. Comptes

Les paramètres d'un compte sont dans le fichier : adresse, nom affiché, serveurs IMAP et SMTP avec leurs identifiants, mode d'authentification, synchronisation de la configuration et identités.

### Identité d'un compte

Un compte est identifié par sa clé, `perso` dans `[accounts.perso]`. La table `account` de la base associe cette clé (`config_key`) à l'identifiant interne référencé par les dossiers, messages et conversations.

- **Renommer la clé à la main crée un nouveau compte**, et l'ancien est désactivé (ci-dessous). Pour renommer un compte en gardant ses données, il faut passer par l'app ou par `nicemailer account rename`, qui modifie le fichier et la base ensemble.
- **Changer l'adresse ou les serveurs** sous la même clé garde le compte. Si la boîte n'est plus la même, la synchronisation le constate par le changement d'`uidvalidity` des dossiers et les retélécharge (`modele de donnees.md`, section 5).

### Un compte absent du fichier est désactivé, pas supprimé

Quand une clé disparaît du fichier, le compte cesse d'être synchronisé, mais ses données restent en base et sur le disque. Sinon, revenir à une ancienne version du fichier dans Git, ou le remplacer par celui d'un autre Mac, effacerait des gigaoctets de mails en cache. Le compte réapparaît intact si sa clé revient.

Supprimer les données d'un compte demande une commande explicite (app, ou `nicemailer account remove <clé>`), qui retire aussi ses secrets du Keychain et sa table du fichier.

### Secrets liés au serveur

Les secrets sont dans le Keychain, sous un nom dérivé de la clé du compte. Chaque entrée enregistre **l'hôte et l'identifiant pour lesquels elle a été créée**. Si le fichier désigne un autre hôte ou un autre identifiant, le noyau refuse de s'authentifier, émet `AccountError` et l'app demande à l'utilisateur de confirmer le nouveau serveur en ressaisissant le mot de passe ou en refaisant l'autorisation OAuth.

Sans cette règle, toute personne ou tout programme capable de modifier le fichier, y compris un agent manipulé par un mail piégé, pourrait rediriger un compte vers son propre serveur et recevoir le mot de passe à la connexion suivante.

**Ajout d'un compte.** Un compte peut être déclaré à la main ou par un agent dans le fichier, mais son secret ne peut être saisi que dans l'app ou le CLI. Tant qu'il manque, le compte est en erreur d'authentification.

### Identités

`identities` liste les adresses d'expédition du compte, en plus de son adresse principale.

- Ce sont les seules adresses proposées et acceptées comme expéditeur d'un brouillon (`brouillons.md`).
- Avec l'adresse principale, elles forment **les adresses du compte lui-même**, exclues de la règle « écrire vaut décision Inbox » (`dossiers.md`) et des correspondants de l'import initial (`import-initial.md`).
- La signature, propre à chaque identité, sera spécifiée avec la composition.

### `authserv_id`

L'identifiant de serveur reconnu dans `Authentication-Results` est détecté automatiquement et enregistré en base (`authentification-expediteur.md`). La clé `authserv_id` d'un compte, si elle est présente, remplace la valeur détectée : c'est le réglage avancé prévu pour un serveur mal détecté.

## 7. Réglages protégés

Certains réglages ne sont **pas** dans le fichier :

| Réglage | Raison |
| --- | --- |
| Droit de modification de l'agent MCP | Un agent qui peut écrire des fichiers pourrait s'accorder lui-même le droit de déplacer ou de supprimer des mails |
| Activation du serveur web | Ouvrir un accès réseau à la boîte ne doit pas dépendre d'un fichier modifiable par n'importe quel programme |

Ils sont stockés en base (table `protected_setting`) et **ne sont modifiables que depuis l'app**, client dont l'identité est vérifiée par sa signature (`architecture.md`, section 9). Le CLI, la TUI, le serveur web et l'agent MCP peuvent les lire, pas les modifier. C'est la même règle que la confirmation des envois demandés par un agent, réservée à l'app.

Limite assumée, comme pour la confirmation des envois : un programme qui a les droits de l'utilisateur peut toujours modifier la base directement. La protection vise les agents qui écrivent des fichiers ou passent par le contrat, pas un logiciel malveillant local.

## 8. Ce qui n'est pas dans le fichier

| Donnée | Stockage | Raison |
| --- | --- | --- |
| Mots de passe, jetons OAuth | Keychain | Secrets |
| Décisions de screening, `auth_policy` | Base (`address`, `domain`) | Données modifiées d'une touche, par milliers, synchronisées entrée par entrée entre Mac |
| Tags, rôles de dossiers | Base (`tag`, `folder`) | Synchronisés entre Mac par `nicemailer-sync`, avec horodatage et pierres tombales |
| Valeurs détectées (`authserv_id`) | Base (`account`) | État, pas intention |
| Réglages protégés | Base (`protected_setting`) | Section 7 |

**Les réglages du fichier ne sont pas synchronisés par `nicemailer-sync`.** Le noyau n'écrit jamais le fichier de lui-même, et un fichier suivi par Git ne doit pas changer sans que son propriétaire l'ait voulu. L'utilisateur qui veut les mêmes réglages sur plusieurs Mac versionne son fichier. Sur une boîte partagée, chaque personne garde ses propres réglages, comme elle garde déjà sa signature.

## 9. Commandes

| Commande | Effet |
| --- | --- |
| `nicemailer config check` | Valide le fichier, sans l'appliquer |
| `nicemailer config defaults` | Affiche le fichier complet, avec toutes les clés, leurs valeurs par défaut et leur rôle en commentaire |
| `nicemailer config path` | Affiche le chemin du fichier utilisé |
| `nicemailer account rename <ancienne> <nouvelle>` | Renomme la clé d'un compte en gardant ses données |
| `nicemailer account remove <clé>` | Supprime un compte, ses données et ses secrets, après confirmation |

## 10. Traçabilité

- Une modification faite par une commande du contrat est inscrite dans `audit_log` (entité `config`, action `update`), avec la source du client, la clé, l'ancienne et la nouvelle valeur.
- Une modification faite directement dans le fichier est constatée au rechargement : action `reload`, source `core`, avec la liste des clés changées. Le noyau ne peut pas savoir qui a édité le fichier ; l'historique Git de l'utilisateur le dit.
- La désactivation et la suppression d'un compte sont inscrites sur l'entité `account` (`disable`, `remove`).

## 11. Catalogue des paramètres

Les paramètres sont définis dans les spécifications qui les utilisent. Ce tableau les rassemble.

| Clé | Rôle | Valeur par défaut | Défini dans |
| --- | --- | --- | --- |
| `core.background_sorting` | Continuer à trier les mails quand NiceMailer est fermé | vrai | `architecture.md`, section 6 |
| `core.idle_exit_secs` | Délai d'arrêt du noyau sans client, quand le tri en arrière-plan est désactivé | 60 | `architecture.md`, section 6 |
| `core.update_timeout_secs` | Durée maximale du mode maintenance | 900 | `mise-a-jour.md` |
| `sync.poll_interval_secs` | Intervalle d'interrogation des dossiers hors Inbox | 300 | `dossiers.md` |
| `config_sync.push_delay_secs` | Regroupement des modifications avant écriture de la configuration synchronisée | 5 | `synchronisation-configuration.md` |
| `events.replay_buffer` | Événements gardés pour le rejeu | 10 000 | `evenements.md` |
| `events.client_queue` | Taille maximale de la file d'un client | 1 000 | `evenements.md` |
| `import.threshold_total` | Seuil de mails de l'Inbox au-delà duquel l'archivage est proposé | 100 | `import-initial.md` |
| `import.threshold_unseen` | Seuil de non-lus au-delà duquel l'archivage est proposé | 50 | `import-initial.md` |
| `import.unseen_default_age_days` | Âge par défaut des non-lus archivés | 30 | `import-initial.md` |
| `import.batch_size` | Nombre d'UID par `UID MOVE` | 1 000 | `import-initial.md` |
| `raw.dir` | Racine du stockage brut | `<data_dir>/raw.noindex` | `stockage-brut.md` |
| `raw.zstd_level` | Niveau de compression | à mesurer | `stockage-brut.md` |
| `raw.fanout` | Niveaux de sous-dossiers | 1 | `stockage-brut.md` |
| `raw.gc_grace_secs` | Délai de grâce des orphelins | 86 400 | `stockage-brut.md` |
| `raw.verify_on_read` | Vérifier le hash à la lecture | vrai | `stockage-brut.md` |
| `ui.view_threshold_ms` | Durée d'affichage avant `mark_viewed` | 2 000 | `tracabilite.md` |
| `audit.retention_days` | Durée de conservation du journal | 365 | `tracabilite.md` |

Un nouveau paramètre est ajouté à la fois dans la spécification qui l'utilise et dans ce tableau.

## 12. Tests exigés

- Fichier absent : démarrage sans compte, valeurs par défaut.
- Fichier invalide au démarrage, puis en cours de route : configuration précédente conservée, `ConfigError` émis avec la bonne ligne.
- Clé inconnue : avertissement, le reste du fichier est appliqué.
- Écriture par `set_config` : commentaires et ordre préservés ; une édition manuelle concurrente n'est pas écrasée.
- Compte retiré du fichier puis remis : données intactes, synchronisation reprise.
- Hôte IMAP modifié dans le fichier : aucune authentification envoyée au nouvel hôte avant confirmation dans l'app.
- `protected_setting` modifié par le CLI ou l'agent MCP : refusé.
- Brouillon avec une adresse d'expédition hors des identités : refusé.
