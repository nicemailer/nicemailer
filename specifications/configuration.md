# NiceMailer : configuration

Statut : validé pour la v1. Complète `architecture.md`, `schema.sql`, `modele de donnees.md` et `synchronisation-configuration.md`.

## 1. Objet

Ce document spécifie où et comment sont stockés les réglages de NiceMailer et les paramètres des comptes.

## 2. Principes

- **La configuration est en deux fichiers texte TOML** : `config.toml` pour les réglages de l'application, `accounts.toml` pour les comptes. L'app offre une interface pour que l'utilisateur novice n'ait jamais à l'ouvrir. Les fichiers texte permettent à l'utilisateur avancé de les versionner dans Git et de les partager entre ses Mac, et à un agent de les modifier simplement.
- **Les fichiers contiennent les intentions de l'utilisateur, la base contient l'état.** Ce que l'utilisateur choisit (un serveur, un intervalle, une identité) est dans les fichiers. Ce qui est détecté (`authserv_id`), calculé ou décidé au fil de l'eau (screening, tags) est en base.
- **Le noyau seul lit et écrit les fichiers.** Les clients passent par le contrat ; l'utilisateur et les agents peuvent aussi les éditer directement.
- **Aucun secret dans les fichiers.** Les mots de passe et les jetons sont dans le Keychain.

## 3. Emplacement et forme

Les fichiers sont `~/.config/nicemailer/config.toml` et `~/.config/nicemailer/accounts.toml`. Ce chemin est celui des outils en ligne de commande et des dépôts de dotfiles ; `~/Library/Application Support`, où vit la base, n'est pas un endroit où l'on pense à chercher un fichier à versionner. La variable d'environnement `NICEMAILER_CONFIG` permet de désigner un autre `config.toml` ; `accounts.toml` est toujours dans le même dossier que lui. Une seule variable déplace ainsi les deux fichiers : un noyau de test ne peut pas lire les vrais comptes par erreur.

### Deux fichiers

- **`config.toml`** contient tous les réglages de l'application (catalogue, section 11).
- **`accounts.toml`** contient les comptes (section 6) et rien d'autre.

Les réglages généraux sont les mêmes d'un Mac à l'autre et peuvent être publiés dans un dépôt de dotfiles ; les comptes (adresses, serveurs, identifiants) sont personnels et diffèrent d'un Mac à l'autre. Les séparer permet de partager les uns sans les autres.

Une table `[accounts…]` dans `config.toml` est une **erreur** et non un avertissement : le fichier est refusé, et le message indique de déplacer la table dans `accounts.toml`. L'ignorer ferait démarrer le noyau sans les comptes de l'utilisateur, sans rien dire.

### Création au démarrage

Au démarrage, le noyau vérifie la présence des deux fichiers et **crée celui qui manque**. Même si l'utilisateur laisse tous les réglages à leur valeur par défaut, il voit ainsi ce qu'il peut modifier :

- `config.toml` est créé avec **toutes les clés, commentées, avec leur valeur par défaut et leur rôle** : le texte de `nicemailer config defaults`, chaque ligne `clé = valeur` précédée de `# `. Les en-têtes de tables (`[sync]`) ne sont pas commentés : pour changer un réglage, il suffit d'enlever le `# ` de sa ligne.
- `accounts.toml` est créé sans compte, avec un exemple de compte commenté.

Commentées plutôt qu'écrites : une valeur écrite dans le fichier est un choix de l'utilisateur et resterait figée si une version ultérieure de NiceMailer changeait la valeur par défaut. Une ligne commentée montre la clé sans rien imposer. Les fichiers créés donnent donc exactement les valeurs par défaut.

- **Un fichier existant n'est jamais modifié**, même vide ou invalide. Un lien symbolique compte comme existant, même si sa cible manque (dépôt de dotfiles pas encore cloné).
- **Complet ou absent.** Le fichier est écrit sous un nom temporaire puis reçoit son nom par un lien (`link`), qui échoue si le nom est pris : un fichier créé entre-temps est conservé, et aucun lecteur ne voit un fichier à moitié écrit.
- **Au démarrage seulement.** Un fichier supprimé pendant que le noyau tourne reste supprimé jusqu'au démarrage suivant.
- Un échec de création (dossier en lecture seule) n'empêche pas le démarrage : le noyau le journalise et démarre avec les valeurs par défaut.

### Forme

- **Une clé absente prend sa valeur par défaut.** Un fichier absent équivaut à un fichier vide : un `config.toml` absent donne toutes les valeurs par défaut, un `accounts.toml` absent aucun compte.
- **Les noms de paramètres des spécifications sont des clés TOML.** `sync.poll_interval_secs` est la clé `poll_interval_secs` de la table `[sync]` de `config.toml`.
- **Réglages par compte.** Un compte est une table `[accounts.<clé>]` d'`accounts.toml`. Le préfixe `accounts.` est gardé : les clés sont ainsi les mêmes partout (erreurs, `ConfigChanged`, `audit_log`). Un réglage général peut y être redéfini pour ce compte (`[accounts.perso.sync]`) ; la valeur du compte l'emporte, et les autres clés viennent du `[sync]` de `config.toml`.
- **Version du format.** Chaque fichier a sa clé `version` (1), qui permet de le migrer si son format change.

Exemple, `config.toml` :

```toml
version = 1

[core]
background_sorting = true

[sync]
poll_interval_secs = 300
```

`accounts.toml` :

```toml
version = 1

[accounts.perso]
email = "prenom@exemple.fr"
display_name = "Prénom Nom"
auth = "password"            # ou "oauth2"
config_sync = false          # synchronisation-configuration.md
read_only = false            # true : ne jamais rien modifier sur le serveur

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

- **Au démarrage**, le noyau crée les fichiers absents (section 3), puis les lit et les valide.
- **En cours de route**, il les surveille (FSEvents) et applique les changements sans redémarrage. Les deux fichiers sont relus ensemble au changement de l'un d'eux : les réglages d'un compte partent du `[sync]` de `config.toml`.
- **Fichier invalide** (syntaxe, type, valeur hors limites) : il est refusé en entier, **avec l'autre fichier**. Le noyau garde la configuration précédente, émet l'événement `ConfigError {file, line, message}` et l'app affiche un bandeau qui renvoie au fichier et à la ligne fautive. Au démarrage, sans configuration précédente, le noyau démarre avec les valeurs par défaut et sans compte, et signale l'erreur de la même façon. Appliquer une partie seulement d'un fichier invalide produirait un état que personne n'a écrit.
- **Clé inconnue** : avertissement, pas erreur. Un fichier écrit par une version plus récente de NiceMailer reste ainsi lisible par une plus ancienne.

## 5. Écriture par l'interface

Les réglages modifiés dans l'app, la TUI ou le CLI le sont par des commandes du contrat (`set_config`, `add_account`…). Le noyau écrit alors le fichier concerné :

- **en préservant commentaires, ordre et mise en forme** (bibliothèque `toml_edit`) : un fichier versionné par l'utilisateur ne doit pas être réécrit de fond en comble pour une clé ;
- **de façon atomique** : fichier temporaire, puis `rename` ;
- **sans écraser une édition manuelle** : si le fichier a changé sur le disque depuis sa dernière lecture, le noyau le relit d'abord, puis n'applique que la clé modifiée ;
- **sans écrire de valeur par défaut** : remettre un réglage à sa valeur par défaut supprime la clé.

Les commandes de comptes (`add_account`, `rename_account`, `remove_account`) écrivent `accounts.toml`, jamais `config.toml`.

En dehors de la création d'un fichier absent au démarrage (section 3), le noyau ne modifie jamais un fichier de sa propre initiative, seulement à la demande d'un client. En particulier, la synchronisation entre Mac ne les modifie pas (section 8).

## 6. Comptes

Les paramètres d'un compte sont dans `accounts.toml` : adresse, nom affiché, serveurs IMAP et SMTP avec leurs identifiants, mode d'authentification, synchronisation de la configuration et identités.

### Identité d'un compte

Un compte est identifié par sa clé, `perso` dans `[accounts.perso]`. La table `account` de la base associe cette clé (`config_key`) à l'identifiant interne référencé par les dossiers, messages et conversations. Les comptes d'archives, créés par un import mbox, sont l'exception : ils n'ont pas de serveur et ne figurent pas dans `accounts.toml` (`import-initial.md`, section 9).

- **Renommer la clé à la main crée un nouveau compte**, et l'ancien est désactivé (ci-dessous). Pour renommer un compte en gardant ses données, il faut passer par l'app ou par `nicemailer account rename`, qui modifie `accounts.toml` et la base ensemble.
- **Changer l'adresse ou les serveurs** sous la même clé garde le compte. Si la boîte n'est plus la même, la synchronisation le constate par le changement d'`uidvalidity` des dossiers et les retélécharge (`modele de donnees.md`, section 5).

### Un compte absent du fichier est désactivé, pas supprimé

Quand une clé disparaît du fichier, le compte cesse d'être synchronisé, mais ses données restent en base et sur le disque. Sinon, revenir à une ancienne version du fichier dans Git, ou le remplacer par celui d'un autre Mac, effacerait des gigaoctets de mails en cache. Le compte réapparaît intact si sa clé revient.

Supprimer les données d'un compte demande une commande explicite (app, ou `nicemailer account remove <clé>`), qui retire aussi ses secrets du Keychain et sa table du fichier.

### Secrets liés au serveur

Les secrets sont dans le Keychain, sous un nom dérivé de la clé du compte. Chaque entrée enregistre **l'hôte et l'identifiant pour lesquels elle a été créée**. Si `accounts.toml` désigne un autre hôte ou un autre identifiant, le noyau refuse de s'authentifier, émet `AccountError` et l'app demande à l'utilisateur de confirmer le nouveau serveur en ressaisissant le mot de passe ou en refaisant l'autorisation OAuth.

Sans cette règle, toute personne ou tout programme capable de modifier `accounts.toml`, y compris un agent manipulé par un mail piégé, pourrait rediriger un compte vers son propre serveur et recevoir le mot de passe à la connexion suivante.

**Ajout d'un compte.** Un compte peut être déclaré à la main ou par un agent dans `accounts.toml`, mais son secret ne peut être saisi que dans l'app ou le CLI. Tant qu'il manque, le compte est en erreur d'authentification.

### Identités

`identities` liste les adresses d'expédition du compte, en plus de son adresse principale.

- Ce sont les seules adresses proposées et acceptées comme expéditeur d'un brouillon (`brouillons.md`).
- Avec l'adresse principale, elles forment **les adresses du compte lui-même**, exclues de la règle « écrire vaut décision Inbox » (`dossiers.md`) et des correspondants de l'import initial (`import-initial.md`).
- La signature, propre à chaque identité, sera spécifiée avec la composition.

### `read_only`

Un compte `read_only = true` n'est jamais modifié sur son serveur : chaque connexion refuse elle-même toute commande d'écriture avant de l'envoyer, aucune action n'est mise dans le spool, les commandes qui modifient un mail sont refusées (`1010`), et le screening tourne « à blanc » (la ligne `screen` de `audit_log` dit où le mail serait allé, rien ne bouge). Usage : regarder une boîte en production sans risque (`plan-imap.md`, phase 8).

### `authserv_id`

L'identifiant de serveur reconnu dans `Authentication-Results` est détecté automatiquement et enregistré en base (`authentification-expediteur.md`). La clé `authserv_id` d'un compte, si elle est présente, remplace la valeur détectée : c'est le réglage avancé prévu pour un serveur mal détecté.

## 7. Réglages protégés

Certains réglages ne sont **pas** dans les fichiers :

| Réglage | Raison |
| --- | --- |
| Droit de modification de l'agent MCP | Un agent qui peut écrire des fichiers pourrait s'accorder lui-même le droit de déplacer ou de supprimer des mails |
| Activation du serveur web | Ouvrir un accès réseau à la boîte ne doit pas dépendre d'un fichier modifiable par n'importe quel programme |

Ils sont stockés en base (table `protected_setting`) et **ne sont modifiables que depuis l'app**, client dont l'identité est vérifiée par sa signature (`architecture.md`, section 9). Le CLI, la TUI, le serveur web et l'agent MCP peuvent les lire, pas les modifier. C'est la même règle que la confirmation des envois demandés par un agent, réservée à l'app.

Limite assumée, comme pour la confirmation des envois : un programme qui a les droits de l'utilisateur peut toujours modifier la base directement. La protection vise les agents qui écrivent des fichiers ou passent par le contrat, pas un logiciel malveillant local.

## 8. Ce qui n'est pas dans les fichiers

| Donnée | Stockage | Raison |
| --- | --- | --- |
| Mots de passe, jetons OAuth | Keychain | Secrets |
| Décisions de screening, `auth_policy` | Base (`address`, `domain`) | Données modifiées d'une touche, par milliers, synchronisées entrée par entrée entre Mac |
| Tags, rôles de dossiers | Base (`tag`, `folder`) | Synchronisés entre Mac par `nicemailer-sync`, avec horodatage et pierres tombales |
| Valeurs détectées (`authserv_id`) | Base (`account`) | État, pas intention |
| Réglages protégés | Base (`protected_setting`) | Section 7 |
| Clé API du fournisseur d'IA | Keychain (`ai:anthropic`) | Secret (`ia.md`, section 3) |
| Prompts | `prompts/*.toml` à côté de `config.toml` | Fichiers à part, un par prompt (`ia.md`, section 4) |

**Les réglages des fichiers ne sont pas synchronisés par `nicemailer-sync`.** Le noyau ne modifie jamais un fichier existant de lui-même, et un fichier suivi par Git ne doit pas changer sans que son propriétaire l'ait voulu. L'utilisateur qui veut les mêmes réglages sur plusieurs Mac versionne son `config.toml`. Sur une boîte partagée, chaque personne garde ses propres réglages, comme elle garde déjà sa signature.

## 9. Commandes

| Commande | Effet |
| --- | --- |
| `nicemailer config check` | Valide les deux fichiers, sans les appliquer |
| `nicemailer config defaults` | Affiche le `config.toml` complet, avec toutes les clés, leurs valeurs par défaut et leur rôle en commentaire |
| `nicemailer config defaults --accounts` | Affiche le modèle d'`accounts.toml`, avec un exemple de compte commenté |
| `nicemailer config path` | Affiche les chemins des deux fichiers utilisés |
| `nicemailer account password <clé>` | Enregistre le mot de passe IMAP du compte dans le trousseau, pour le serveur que nomme `accounts.toml` ; le refaire confirme un nouveau serveur |
| `nicemailer account rename <ancienne> <nouvelle>` | Renomme la clé d'un compte en gardant ses données |
| `nicemailer account remove <clé>` | Supprime un compte, ses données et ses secrets, après confirmation |
| `nicemailer account add [<clé>] --email … [--host …]` | Crée un compte : connexion d'essai, rôles des dossiers, import initial, mot de passe, table d'`accounts.toml` (`import-initial.md`). Sans clé, le premier compte d'un domaine prend le nom du domaine, sans suffixe public ni sous-domaine (`pierre.dupont@toto-mail.com` → `toto-mail`) ; s'il existe déjà un compte de ce domaine, ou si ce nom est la clé d'un autre compte, la clé est tirée de l'adresse entière (`pierre_dupont_toto_mail_com`). Sans `--host`, les serveurs sont cherchés dans la base des fournisseurs (celle de Thunderbird, convertie par `make autoconfig`, et la nôtre, `data/providers/local/`), par le domaine de l'adresse puis par ses MX (un domaine hébergé chez Google est reconnu à `aspmx.l.google.com`). Le serveur SMTP est essayé dans la foulée : ceux du fournisseur, sinon, pour un `--host` donné, ses ports standard (465, 587) et le même nom avec `imap` remplacé par `smtp` ; le premier qui fonctionne est écrit. Chaque étape est affichée : rien n'est choisi sans dire pourquoi |
| `nicemailer account roles <clé> rôle=dossier…` | Change le rôle de dossiers du compte, en créant au besoin ceux qui manquent |

## 10. Traçabilité

- Une modification faite par une commande du contrat est inscrite dans `audit_log` (entité `config`, action `update`), avec la source du client, la clé, l'ancienne et la nouvelle valeur.
- Une modification faite directement dans un fichier est constatée au rechargement : action `reload`, source `core`, avec la liste des clés changées. Le noyau ne peut pas savoir qui a édité le fichier ; l'historique Git de l'utilisateur le dit.
- La désactivation et la suppression d'un compte sont inscrites sur l'entité `account` (`disable`, `remove`).

## 11. Catalogue des paramètres

Les paramètres sont définis dans les spécifications qui les utilisent. Ce tableau les rassemble.

| Clé | Rôle | Valeur par défaut | Défini dans |
| --- | --- | --- | --- |
| `core.background_sorting` | Continuer à trier les mails quand NiceMailer est fermé | vrai | `architecture.md`, section 6 |
| `core.idle_exit_secs` | Délai d'arrêt du noyau sans client, quand le tri en arrière-plan est désactivé | 60 | `architecture.md`, section 6 |
| `core.update_timeout_secs` | Durée maximale du mode maintenance | 900 | `mise-a-jour.md` |
| `sync.poll_interval_secs` | Intervalle d'interrogation des dossiers hors Inbox | 300 | `dossiers.md` |
| `sync.max_connections` | Connexions IMAP par compte : 2 (dossiers et IDLE sur l'une, corps sur l'autre) ou 1 (tout sur une seule, pour un serveur qui en permet peu) | 2 | `plan-imap.md`, phase 4 |
| `config_sync.push_delay_secs` | Regroupement des modifications avant écriture de la configuration synchronisée | 5 | `synchronisation-configuration.md` |
| `events.replay_buffer` | Événements gardés pour le rejeu | 10 000 | `evenements.md` |
| `events.client_queue` | Taille maximale de la file d'un client | 1 000 | `evenements.md` |
| `import.threshold_total` | Seuil de mails de l'Inbox au-delà duquel l'archivage est proposé | 100 | `import-initial.md` |
| `import.threshold_unseen` | Seuil de non-lus au-delà duquel l'archivage est proposé | 50 | `import-initial.md` |
| `import.unseen_default_age_days` | Âge par défaut des non-lus archivés | 30 | `import-initial.md` |
| `import.batch_size` | Nombre d'UID par `UID MOVE` | 1 000 | `import-initial.md` |
| `raw.dir` | Racine du stockage brut | `<data_dir>/raw.noindex` | `stockage-brut.md` |
| `raw.zstd_level` | Niveau de compression | à mesurer | `stockage-brut.md` |
| `raw.fanout` | Niveaux de sous-dossiers ; reste à 1, un second niveau n'apporte rien de mesurable (`stockage-brut.md`, section 7) | 1 | `stockage-brut.md` |
| `raw.gc_grace_secs` | Délai de grâce des orphelins | 86 400 | `stockage-brut.md` |
| `raw.verify_on_read` | Vérifier le hash à la lecture | vrai | `stockage-brut.md` |
| `ui.view_threshold_ms` | Durée d'affichage avant `mark_viewed` | 2 000 | `tracabilite.md` |
| `audit.retention_days` | Durée de conservation du journal | 365 | `tracabilite.md` |
| `ai.enabled` | Autoriser l'envoi de mails au fournisseur d'IA, pour tous les comptes | faux | `ia.md` |
| `ai.provider` | Fournisseur : `claude-code` (abonnement, binaire local) ou `anthropic-api` (clé dans le Keychain) | `claude-code` | `ia.md` |
| `ai.model` | Modèle par défaut, qu'un prompt peut remplacer | `claude-sonnet-5-5` | `ia.md` |
| `ai.claude_path` | Chemin du binaire `claude` ; vide : recherché | vide | `ia.md` |
| `ai.timeout_secs` | Délai maximal d'une exécution | 120 | `ia.md` |
| `ai.max_parallel` | Exécutions simultanées | 2 | `ia.md` |
| `ai.batch_size` | Conversations par appel pour un lot | 20 | `ia.md` |
| `ai.max_folder_items` | Conversations au plus quand un prompt traite tout le dossier | 500 | `ia.md` |
| `ai.history_days` | Durée avant la purge du contenu de l'historique | 30 | `ia.md` |

Un nouveau paramètre est ajouté à la fois dans la spécification qui l'utilise et dans ce tableau.

## 12. Tests exigés

- Fichiers absents au démarrage : créés, `config.toml` avec toutes les clés commentées, `accounts.toml` sans compte ; ils donnent les valeurs par défaut et aucun compte. Décommenter n'importe quelle ligne donne un fichier valide.
- Fichier existant, même vide ou invalide, ou lien symbolique sans cible : jamais modifié.
- Fichier supprimé en cours de route : valeurs par défaut (ou aucun compte), pas de recréation avant le démarrage suivant.
- Table `[accounts…]` dans `config.toml` : refusée, avec un message qui renvoie à `accounts.toml`.
- `accounts.toml` invalide avec un `config.toml` valide : les deux refusés, `ConfigError` nomme `accounts.toml`.
- Fichier invalide au démarrage, puis en cours de route : configuration précédente conservée, `ConfigError` émis avec le bon fichier et la bonne ligne.
- Clé inconnue : avertissement, le reste du fichier est appliqué.
- Écriture par `set_config` : commentaires et ordre préservés ; une édition manuelle concurrente n'est pas écrasée.
- Compte retiré du fichier puis remis : données intactes, synchronisation reprise.
- Hôte IMAP modifié dans `accounts.toml` : aucune authentification envoyée au nouvel hôte avant confirmation dans l'app.
- `protected_setting` modifié par le CLI ou l'agent MCP : refusé.
- Brouillon avec une adresse d'expédition hors des identités : refusé.
