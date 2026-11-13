# NiceMailer : synchronisation de la configuration entre Mac

Statut : validé. Complète `dossiers.md` (screening) et `modele de donnees.md`.

## 1. Objet

Ce document spécifie comment plusieurs Mac utilisant NiceMailer sur une même boîte partagent leurs décisions de screening et la configuration de cette boîte.

Deux cas d'usage :

1. **Un utilisateur, plusieurs Mac** : un ordinateur au bureau, un autre à la maison, qui travaillent sur les mêmes boîtes.
2. **Une boîte fonctionnelle partagée** (contact, info, support) traitée par plusieurs personnes, chacune sur son Mac.

Le produit s'adresse à des individus et à de petites structures. La synchronisation reste donc volontairement simple, en « best effort ».

### Ce qui reste hors périmètre

- **Mac éteint.** Le screening n'a lieu que si un noyau NiceMailer tourne sur au moins un Mac connecté à la boîte. Quand aucun ne tourne, les mails restent dans l'Inbox du serveur et sont triés au prochain démarrage. C'est le comportement attendu.
- **Règles Sieve.** Générer des règles Sieve côté serveur est écarté : les capacités du serveur distant sont inconnues (ManageSieve absent, extensions variables), Sieve ne sait pas exploiter le verdict d'authentification (`authentification-expediteur.md`), et la plupart des utilisateurs perdraient toute traçabilité sur ce que fait le serveur.

## 2. Principes

- **La configuration est stockée sur le serveur IMAP de chaque boîte**, dans un dossier dédié. Aucun service tiers, aucun compte Apple ou NiceMailer n'est nécessaire.
- **Activation explicite par compte** (clé `config_sync` du compte dans `accounts.toml`), désactivée par défaut : on écrit sur le serveur une donnée que lisent tous ceux qui ont accès à la boîte, administrateur compris.
- **La dernière modification l'emporte, entrée par entrée.** Chaque entrée (une adresse, un domaine, un rôle de dossier, un tag…) porte la date de sa dernière modification. Il n'y a ni poste principal, ni mode lecture seule, ni résolution manuelle des conflits.
- **Les décisions de screening ont une portée globale.** Une adresse a la même décision sur toutes les boîtes (`address` et `domain` restent communes à tous les comptes). Une décision prise sur une boîte partagée s'applique donc aussi aux boîtes personnelles, et inversement. Une adresse classée différemment selon la boîte est un cas jugé marginal pour la cible du produit.
- **Confidentialité : une boîte ne reçoit que les décisions sur des adresses qu'elle a déjà vues** (section 6). Le carnet d'adresses complet n'est jamais divulgué à une boîte où l'on collabore ponctuellement.
- **Best effort.** Les Mac convergent en quelques minutes. Pendant ce délai, un Mac peut encore trier selon une règle périmée ; ce n'est pas corrigé après coup (section 9).

## 3. Contenu synchronisé

| Synchronisé | Clé de l'entrée | Remarque |
| --- | --- | --- |
| Décision de screening d'une adresse | adresse | Soumis au filtre de confidentialité |
| Décision de screening d'un domaine | domaine | Soumis au filtre de confidentialité |
| `auth_policy` d'une adresse | adresse | Choix de l'utilisateur, comme le screening ; même filtre |
| Provenance forcée d'une adresse (`sender_kind_user`, `expediteurs.md`) | adresse | Choix de l'utilisateur ; même filtre. Le verdict calculé ne l'est pas : il se recalcule à partir des messages |
| Rôle des dossiers (Archive → « Archives », Junk → « Spam »…), dont les dossiers du workflow créés par NiceMailer | rôle | Propre à la boîte |
| Tags : nom et couleur associés à un mot-clé IMAP | mot-clé | Propre à la boîte (`tag.account_id`) |
| Groupes de contacts : nom, couleur, position | `sync_key` du groupe | Commun à tous les comptes ; écrit seulement s'il a une appartenance écrite (section 6, `groupes.md`) |
| Appartenance d'une adresse à un groupe | `sync_key` et adresse | Soumis au filtre de confidentialité |

| Non synchronisé | Raison |
| --- | --- |
| Identifiants, serveurs, références Keychain | Secrets et paramètres propres au poste |
| Réglages de `config.toml` et d'`accounts.toml` (généraux et par compte) | Le noyau ne modifie jamais ces fichiers de lui-même (il ne fait que créer ceux qui manquent au démarrage) : un fichier que l'utilisateur versionne dans Git ne doit pas changer sans lui. Les mêmes réglages sur plusieurs Mac s'obtiennent en versionnant `config.toml` (`configuration.md`, section 8) |
| `auth_seen` | Se recalcule à partir des messages, qui sont les mêmes sur tous les Mac |
| Signature, nom d'affichage de l'expéditeur | Propres à chaque personne sur une boîte partagée |
| Option « continuer à trier mes mails quand NiceMailer est fermé », préférences d'affichage | Propres au poste |
| `address.name` (nom d'affichage d'un contact) | Pas une décision de screening ; à reconsidérer si le besoin apparaît |

## 4. Stockage sur le serveur

### Le dossier

- Nom : `nicemailer-sync`, créé à l'activation, au premier niveau de l'espace de noms personnel du serveur. Le préfixe et le séparateur sont ceux annoncés par le serveur (par exemple `INBOX.nicemailer-sync` sur certains Dovecot ou Courier).
- Le nom est volontairement technique, en minuscules et avec un tiret, pour ne pas entrer en collision avec un dossier `NiceMailer` que l'utilisateur aurait créé lui-même. Le tiret ne demande aucun encodage IMAP.
- Si un dossier `nicemailer-sync` existe déjà et contient d'autres messages, ceux qui ne portent pas `X-NiceMailer-Config` sont ignorés et jamais supprimés.
- En base, ce dossier a le rôle `config` (`folder.role`).
- Il est **exclu de tout le reste** : pas de screening, pas de ligne `message`, pas de conversation, pas d'index plein texte, pas d'événement, pas d'exposition aux clients. Le moteur lit ses messages directement par `UID FETCH`.

### Le message de configuration

- Un message par boîte, identifié par l'en-tête `X-NiceMailer-Config: 1`, où `1` est la **version du format**. Objet lisible : « Configuration NiceMailer – ne pas supprimer ».
- Corps : un court texte expliquant ce qu'est ce message, et une pièce jointe `nicemailer-config.json` (`application/json`).
- Marqué `\Seen`, pour ne pas compter dans les non-lus d'un autre client.
- Il n'est jamais envoyé : il est déposé par `APPEND`.

Taille indicative : une centaine d'octets par entrée, soit environ 1 Mo pour 10 000 adresses.

### Format du document

```json
{
  "format": 1,
  "written_at": 1760870400,
  "addresses": {
    "info@mabanque.fr":          {"screening": "newsletter", "screening_at": 1760000000},
    "releve@mabanque.fr":        {"screening": "paperwork",  "screening_at": 1760000100,
                                  "auth_policy": "required", "auth_policy_at": 1760000100},
    "mon.conseiller@mabanque.fr":{"screening": "inbox",      "screening_at": 1760000200},
    "ancien@exemple.fr":         {"screening": null,         "screening_at": 1760500000}
  },
  "domains": {
    "promo-spam.example":        {"screening": "banned",     "screening_at": 1759000000}
  },
  "folder_roles": {
    "archive":  {"path": "Archives",          "at": 1758000000},
    "junk":     {"path": "Spam",              "at": 1758000000},
    "todo":     {"path": "Todo",              "at": 1758000000}
  },
  "tags": {
    "client":   {"name": "Client",  "color": "#3b82f6", "at": 1758500000},
    "urgent":   {"name": null,      "at": 1759500000}
  },
  "groups": {
    "g_3fa9c2d41b0e7a55": {"name": "VIP", "color": "#f59e0b", "position": 0, "at": 1760000000}
  },
  "group_members": {
    "g_3fa9c2d41b0e7a55": {
      "patron@entreprise.fr": {"member": true, "at": 1760000100}
    }
  },
  "settings": {}
}
```

- Chaque entrée porte l'horodatage de sa dernière modification, en secondes Unix UTC.
- **Une valeur `null` est une suppression** (une « pierre tombale ») : la décision ou le tag a été retiré à cette date. Sans elle, une entrée supprimée sur un Mac reviendrait depuis un autre. Les pierres tombales ne sont jamais purgées : leur taille est négligeable.
- Les champs inconnus sont conservés tels quels à la réécriture (section 8).

## 5. Fusion

La fusion porte sur chaque entrée indépendamment, et pour une adresse sur chaque attribut (`screening` et `auth_policy` ont chacun leur horodatage).

1. L'entrée la plus récente l'emporte.
2. À horodatage égal, la plus grande valeur dans l'ordre lexicographique l'emporte (`null` étant la plus petite). Ce départage arbitraire est déterministe : tous les Mac aboutissent au même résultat.

La fusion donne le même résultat quel que soit l'ordre dans lequel on fusionne les documents. Les écritures concurrentes de deux Mac ne posent donc pas de problème (section 7).

**Horloges.** La comparaison repose sur l'horloge de chaque Mac. Un Mac dont l'horloge avance l'emporte à tort pendant l'écart. macOS synchronisant l'heure par NTP, la limite est assumée.

## 6. Confidentialité

Une entrée d'adresse n'est écrite dans le document d'une boîte que si **l'adresse a été vue dans cette boîte** : elle figure en base comme expéditeur (`message.from_address_id`, index `idx_message_from`) ou destinataire (`message_address`) d'au moins un message du compte.

Une entrée de domaine n'est écrite que si au moins une adresse de ce domaine a été vue dans la boîte.

Le filtre s'applique au moment de l'écriture, sur les entrées locales. Les entrées déjà présentes sur le serveur sont conservées à la réécriture, même si l'adresse n'est plus vue localement (messages supprimés depuis) : la boîte les connaissait déjà.

Conséquences assumées de la portée globale :

- sur une boîte partagée, les collègues voient la décision que j'ai prise pour une adresse qui y a écrit (par exemple que `releve@mabanque.fr` va dans Paperwork) ;
- une décision reçue d'une boîte partagée s'applique à mes autres boîtes. Elle n'est republiée vers mes autres boîtes synchronisées que si l'adresse y a été vue.

Les rôles de dossiers et les tags sont propres à la boîte et ne sont pas filtrés.

Une appartenance à un groupe n'est écrite que si l'adresse a été vue dans la boîte, et un groupe que s'il a au moins une appartenance écrite dans ce document ou s'il y figure déjà : les collègues d'une boîte partagée ne voient pas un groupe « Famille » sans rapport avec elle (`groupes.md`, section 4).

## 7. Déroulement

### Lecture

À chaque interrogation périodique des dossiers (`sync.poll_interval_secs`, 300 s par défaut, `dossiers.md`), le moteur contrôle le dossier `nicemailer-sync` par `STATUS` (`UIDNEXT`, et `HIGHESTMODSEQ` si CONDSTORE est disponible). S'il n'a pas changé depuis la dernière lecture, rien n'est fait.

Sinon, le moteur lit **tous** les messages portant `X-NiceMailer-Config`, fusionne leurs documents (section 5), puis applique le résultat en base dans une transaction :

- une entrée distante plus récente que l'entrée locale la remplace (une adresse ou un domaine inconnu est créé) ;
- le rôle d'un dossier n'est appliqué que si ce chemin existe dans la liste des dossiers du compte ; sinon il est réessayé après la prochaine synchronisation de la liste ;
- un `AccountChanged` est émis pour le compte si ses rôles ou ses tags ont changé.

Une décision de screening reçue ne déclenche pas le rangement des anciens mails de l'expéditeur (`dossiers.md`, « Décision sur un expéditeur ») : le Mac qui a pris la décision les a déjà rangés sur le serveur, et la synchronisation IMAP rapporte ces déplacements. Sur une adresse vue uniquement localement, les mails restent où ils sont.

### Écriture

Une écriture est nécessaire quand une entrée locale (après filtre de confidentialité) est plus récente que celle du serveur, ou quand le dossier contient plusieurs messages de configuration.

Toute modification locale d'un élément synchronisé (décision, `auth_policy`, rôle, tag, groupe, appartenance) programme une écriture, regroupée pendant `config_sync.push_delay_secs` (5 s). L'écriture passe par le spool, comme tout ordre envoyé au serveur : action `push_config`, sans message, dont le contenu est calculé **au moment de l'exécution** et non à l'insertion. Une seule action `push_config` en attente par compte : une nouvelle demande est fusionnée avec elle.

Exécution :

1. Lire les messages de configuration présents et les fusionner (comme en lecture), en notant leurs UID.
2. Fusionner le résultat avec les entrées locales filtrées.
3. `APPEND` du nouveau message (flag `\Seen`).
4. `UID STORE +FLAGS (\Deleted)` puis `UID EXPUNGE` des **seuls UID lus à l'étape 1**.

Si deux Mac écrivent en même temps, chacun ne supprime que ce qu'il a lu : il reste deux messages, que la lecture suivante fusionne puis remplace par un seul. Aucune modification n'est perdue.

L'application d'entrées reçues ne déclenche pas d'écriture : seules les modifications locales en déclenchent. Deux Mac ne se renvoient donc pas indéfiniment le même document.

### Activation et désactivation

- **Activation** : création du dossier `nicemailer-sync` s'il n'existe pas, lecture, puis écriture. Si la boîte a déjà une configuration (un collègue l'a activée avant), la fusion intègre les deux sens : rien n'est écrasé sans être plus récent.
- **Désactivation** : le Mac cesse de lire et d'écrire. Le dossier et son message restent sur le serveur pour les autres Mac.

## 8. Robustesse

- **Document illisible** (JSON invalide, pièce jointe absente) : ignoré et journalisé. Il n'est jamais supprimé ; seuls les UID d'un document lu avec succès sont supprimés à l'étape 4.
- **Format plus récent que celui que le noyau comprend** (`X-NiceMailer-Config` supérieur) : le noyau applique les sections qu'il connaît mais **n'écrit plus** sur cette boîte tant qu'il n'est pas mis à jour, pour ne pas détruire des données qu'il ne sait pas lire. L'app le signale dans les réglages du compte.
- **Champs inconnus** dans un format connu : conservés à la réécriture.
- **Message supprimé par un autre client** : la lecture suivante ne trouve rien ; le prochain Mac qui a une modification locale réécrit le document à partir de sa base.

## 9. Screening sur plusieurs Mac

Plusieurs Mac peuvent trier la même Inbox en même temps.

- **Déplacements concurrents.** Deux Mac voient le même mail et lancent chacun un `UID MOVE`. Le second constate que le message a disparu du dossier source : c'est traité comme un **succès**, pas comme un échec à annuler (`modele de donnees.md`, section 6, « Message disparu du serveur »).
- **Règle pas encore reçue.** Si un Mac n'a pas encore lu une décision prise sur un autre, un mail de cet expéditeur peut aller en Quarantine pendant au plus un intervalle d'interrogation. Ce n'est pas corrigé : c'est la limite du best effort.

## 10. Données

```sql
-- account : l'activation est la clé config_sync du compte dans accounts.toml
-- (faux par défaut), pas une colonne (configuration.md).

-- folder : nouveau rôle 'config' (dossier nicemailer-sync) ;
-- horodatage de l'attribution du rôle
date_role_modified INTEGER,

-- address : date_screening_modified existe déjà ; target NULL avec une date = décision retirée
date_auth_policy_modified INTEGER,

-- tag : date_modified (colonne commune de traçabilité) sert d'horodatage d'entrée
date_deleted INTEGER,         -- pierre tombale : tag supprimé, masqué partout

-- contact_group : sync_key (clé d'entrée), date_modified comme horodatage,
-- date_deleted (pierre tombale) ; contact_group_member : date_modified,
-- date_removed (pierre tombale). Voir groupes.md.

-- pending_action : nouveau kind 'push_config' (message_id NULL)
```

Une modification reçue par la synchronisation est enregistrée avec `modified_by = 'config_sync'` (ou `created_by` pour une entrée créée), et les dates prennent l'horodatage de l'entrée reçue, pas l'heure locale : sinon, la fusion suivante considérerait à tort la copie locale comme plus récente. Elle est aussi inscrite dans `audit_log` (`tracabilite.md`).

## 11. Paramètres

Ces paramètres sont des clés de `config.toml` ; le catalogue complet est dans `configuration.md`, section 11.

| Paramètre | Rôle | Valeur initiale |
| --- | --- | --- |
| `config_sync.push_delay_secs` | Regroupement des modifications locales avant écriture | 5 |
| `sync.poll_interval_secs` | Intervalle de lecture du dossier `nicemailer-sync` (paramètre commun) | 300 |

## 12. Tests exigés

- Fusion : entrée la plus récente retenue ; égalité départagée de façon identique quel que soit l'ordre des documents.
- Pierre tombale : une décision retirée sur un Mac ne revient pas depuis un autre.
- Écritures concurrentes de deux Mac : deux messages coexistent, puis un seul après la lecture suivante, sans perte.
- Filtre : une décision sur une adresse jamais vue dans la boîte n'est pas écrite ; un domaine sans adresse vue non plus.
- Entrée distante sur une adresse inconnue : adresse créée avec sa décision.
- Rôle reçu pour un dossier absent localement : appliqué après la synchronisation de la liste des dossiers.
- Document illisible : ignoré, jamais supprimé.
- Format plus récent : lecture des sections connues, aucune écriture.
- Application d'entrées reçues : aucune écriture déclenchée.
- Dossier `nicemailer-sync` : absent des listes, de la recherche et du screening.
- Déplacement concurrent de deux Mac : le second `move` passe à `done`, aucune annulation locale.
