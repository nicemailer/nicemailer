# NiceMailer : brouillons

Statut : validé pour la v1. Complète `dossiers.md` (dossier Drafts, réponse et rangement), `schema.sql`, `modele de donnees.md` et `stockage-brut.md`.

## 1. Objet

Ce document spécifie comment un brouillon est rédigé, enregistré, synchronisé avec le serveur IMAP et envoyé. Il couvre aussi les pièces jointes à envoyer.

## 2. Principes

- **Le brouillon vit en local pendant la rédaction.** Il est enregistré dans la table `draft`, sous une forme éditable, pas en MIME. Chaque modification y est écrite, et rien ne part sur le serveur pendant la rédaction.
- **Le serveur reçoit le brouillon à la fermeture de la fenêtre de rédaction.** La copie dans le dossier Drafts du serveur permet de retrouver le brouillon sur le téléphone, et inversement. C'est un reflet du brouillon local, pas sa source.
- **Un texte rédigé n'est jamais perdu en silence.** Quand le même brouillon a été modifié des deux côtés, les deux versions sont gardées (section 6).
- **Le `Message-ID` est stable.** Il est généré à la création du brouillon et ne change plus : toutes les versions envoyées au serveur et le mail final le portent. C'est lui qui relie les versions entre elles malgré les changements d'UID.
- **Les pièces jointes sont copiées dans la zone de stockage privée** dès qu'on les attache. Le brouillon ne dépend plus du fichier d'origine.
- **Toute la logique est dans le noyau.** Les clients n'ont que des commandes (section 8), conformément au principe « aucune logique métier côté Swift » (`architecture.md`).

## 3. Données

### `draft`

Une ligne par brouillon en cours.

| Colonne | Rôle |
| --- | --- |
| `account_id`, `from_addr`, `from_name` | Compte et identité d'envoi |
| `recipients` | JSON : liste de `{kind, addr, name}`, `kind` parmi `to`, `cc`, `bcc` |
| `subject`, `body`, `body_format` | Contenu en cours d'édition |
| `reply_to_message_id` | Le mail auquel on répond ; NULL pour un nouveau mail. `In-Reply-To` et `References` en sont déduits à l'envoi |
| `after_send` | Rangement choisi pour la conversation après l'envoi : `archive`, `waiting` ou `todo` (`dossiers.md`, « Répondre à une conversation »). NULL : la présélection du noyau s'applique |
| `message_id_hdr` | `Message-ID` stable (section 2) |
| `message_id` | La ligne `message` qui représente la copie du brouillon sur le serveur ; NULL tant qu'aucune copie n'existe |
| `revision` | Incrémentée à chaque modification locale |
| `server_revision` | Révision de la dernière copie envoyée au serveur ; NULL si aucune |
| `state` | `editing` : modifiable ; `sending` : figé, en cours d'envoi |
| traçabilité | `date_created`, `date_modified`, `created_by`, `modified_by` (`tracabilite.md`) |

Le brouillon est **modifié localement** quand `revision` diffère de `server_revision`.

Les modifications d'un brouillon ne sont pas inscrites dans `audit_log` : ce serait une ligne par pause de frappe. L'envoi, lui, est journalisé (`send`).

### `draft_attachment`

Les pièces jointes d'un brouillon : nom affiché, type MIME, taille, ordre, `content_id` et `is_inline` pour les images insérées dans le corps HTML, et `hash`, qui désigne le fichier dans la zone de stockage privée (section 7).

### La ligne `message` d'un brouillon

Un brouillon envoyé au serveur est aussi représenté par **une seule ligne `message`** dans le dossier Drafts, avec `is_draft = 1`. C'est elle qu'affiche la liste du dossier Drafts, comme pour les autres dossiers.

- **Une ligne par brouillon, pas par version.** À chaque nouvelle copie envoyée au serveur, la même ligne est mise à jour (`uid`, objet, aperçu), comme pour un déplacement : son identité ne change pas.
- **Pas de brut pour nos propres brouillons.** La synchronisation reconnaît l'UID qu'elle vient d'écrire et ne télécharge pas le message : la ligne est remplie depuis `draft`. Le brut d'un brouillon serait régénérable, et le mail envoyé aura son brut normalement, par Sent. Aucun blob n'est donc créé puis abandonné à chaque version.

## 4. Cycle de vie

1. **Création** (`create_draft`) : une ligne `draft`, avec un `Message-ID` neuf. Pour une réponse, les destinataires, l'objet et la citation sont préparés par le noyau.
2. **Rédaction** : le client ouvre le brouillon (`open_draft`), puis envoie ses modifications (`update_draft`) à chaque pause de frappe. Elles sont écrites dans `draft` ; rien ne part sur le serveur.
3. **Fermeture** (`close_draft`) : si le brouillon a été modifié localement, le noyau met en file une action `append_draft` (section 5).
4. **Reprise** : rouvrir un brouillon fermé, depuis NiceMailer ou après l'avoir retrouvé dans le dossier Drafts, reprend les étapes 2 et 3.
5. **Envoi** (`send_draft`) : section 9.
6. **Abandon** (`discard_draft`) : la ligne `draft` est supprimée, et la copie sur le serveur l'est aussi s'il y en a une.

**Fenêtre restée ouverte.** Tant que la fenêtre de rédaction est ouverte, le téléphone ne voit pas la dernière version : c'est le prix du choix d'envoyer au serveur à la fermeture seulement.

**Plantage ou arrêt pendant la rédaction.** Le brouillon est déjà en base, à la dernière modification reçue. Au redémarrage, aucun brouillon n'est considéré comme ouvert : un brouillon modifié localement mais pas encore envoyé au serveur est envoyé comme à une fermeture.

**Clients sans fenêtre** (CLI, agent MCP). Une commande qui crée ou modifie un brouillon vaut fermeture : le brouillon est envoyé au serveur aussitôt après.

## 5. Envoi d'une copie au serveur (`append_draft`)

IMAP ne permet pas de modifier un message : une nouvelle version est un nouveau message. L'action du spool :

1. **Vérifie que la copie précédente existe toujours** (`server_revision` non NULL) : son UID est-il encore dans Drafts ? S'il a disparu sans que NiceMailer l'ait supprimé, le brouillon a été modifié, envoyé ou supprimé ailleurs : la section 6 s'applique, et l'action s'arrête là.
2. **Construit le MIME** de la révision courante, avec le `Message-ID` stable et l'en-tête `X-NiceMailer-Draft-Revision`, et l'écrit dans Drafts par `APPEND`, avec les flags `\Draft` et `\Seen`.
3. **Enregistre le nouvel UID** (réponse `APPENDUID`, extension UIDPLUS ; à défaut, recherche du `Message-ID` dans Drafts) dans la ligne `message`, et `server_revision = revision`.
4. **Supprime la copie précédente** (`\Deleted`, puis `UID EXPUNGE` sur cet UID).

L'ordre « écrire la nouvelle version, puis supprimer l'ancienne » garantit qu'il y a toujours au moins une copie sur le serveur. Un plantage entre les étapes 2 et 4 laisse au pire deux copies.

**Reprise après plantage.** L'action doit pouvoir être rejouée. Avant l'`APPEND`, elle cherche le `Message-ID` dans Drafts : une copie qui porte déjà la révision courante (`X-NiceMailer-Draft-Revision`) n'est pas réécrite, et les copies plus anciennes sont supprimées. C'est pour cela que la révision est écrite dans un en-tête : elle rend l'action rejouable sans doublon. L'en-tête est retiré du mail envoyé.

## 6. Modifications sur un autre appareil

IMAP n'a pas de verrou : on ne peut pas empêcher deux appareils de modifier le même brouillon, seulement le détecter. Une modification faite ailleurs se voit à la disparition de la copie écrite par NiceMailer, soit à la synchronisation de Drafts, soit à l'étape 1 de `append_draft`. Trois cas :

| Modifié localement ? | Modifié ailleurs ? | Effet |
| --- | --- | --- |
| Non | Oui | **Adoption** : la version du serveur remplace le contenu local, sans rien demander. C'est le cas courant : commencé sur le Mac, poursuivi sur le téléphone, repris sur le Mac |
| Oui | Non | Envoi normal de la nouvelle version (section 5) |
| Oui | Oui | **Conflit** : les deux versions sont gardées comme deux brouillons distincts. La version locale reçoit un nouveau `Message-ID` et part sur le serveur comme un nouveau brouillon. L'app affiche « Ce brouillon a aussi été modifié sur un autre appareil » sur les deux |

**Retrouver la version modifiée ailleurs.** Le noyau cherche dans Drafts un message de même `Message-ID`. Le comportement des autres clients reste à vérifier : s'ils génèrent un nouveau `Message-ID` à chaque enregistrement, la version modifiée apparaît comme un brouillon étranger sans lien avec le nôtre. Le cas « modifié ailleurs » se réduit alors à « disparu » (ci-dessous), et l'utilisateur voit deux brouillons dans Drafts, sans perte de texte.

**Copie disparue sans remplaçant.** Le brouillon a été envoyé ou supprimé ailleurs.

- Le noyau cherche son `Message-ID` dans Sent. S'il l'y trouve, le brouillon a été envoyé depuis un autre appareil : la ligne `draft` est supprimée.
- Sinon, si le brouillon n'a **pas** été modifié localement, la suppression faite ailleurs est respectée : la ligne `draft` est supprimée. Elle ne contenait rien que l'utilisateur n'ait vu sur l'autre appareil.
- Sinon (modifié localement), le texte local est neuf : il repart sur le serveur comme un nouveau brouillon, avec un avertissement.

**Brouillon ouvert pendant la modification.** Le noyau ne remplace jamais le contenu d'un brouillon ouvert dans une fenêtre : l'utilisateur est en train de le lire ou de l'écrire. La modification faite ailleurs est traitée à la fermeture, selon le tableau. Le noyau tient la liste des brouillons ouverts en mémoire (`open_draft`, `close_draft`) : elle n'a pas de sens après un redémarrage.

**Fréquence.** Drafts est interrogé comme les autres dossiers, toutes les cinq minutes, ainsi qu'à l'ouverture d'un brouillon, pour rouvrir la dernière version.

**Brouillon créé ailleurs.** Un brouillon rédigé sur le téléphone arrive comme une ligne `message` ordinaire, avec son brut. À son ouverture dans NiceMailer, le noyau analyse son MIME, crée la ligne `draft` liée à cette ligne `message` et reprend son `Message-ID`. Le brouillon devient alors un brouillon NiceMailer comme les autres : sa prochaine copie est écrite par `append_draft`, et le brut d'origine n'est plus référencé.

## 7. Pièces jointes à envoyer

Attacher un fichier (`add_draft_attachment`) le **copie aussitôt** dans la zone de stockage privée du noyau :

```
<data_dir>/
  outgoing.noindex/        ← pièces jointes des brouillons
    ab/
      abcdef…0123          ← hash SHA-256 du contenu, sans compression
```

- **Adressage par le contenu**, comme le brut : un même fichier joint à deux brouillons n'est stocké qu'une fois. Les fichiers sont en `0400`, les dossiers en `0700`. Pas de compression : les pièces jointes courantes (PDF, images, archives) se compressent mal.
- **Indépendance du fichier d'origine.** Un brouillon peut vivre des jours ; le fichier d'origine peut être déplacé, modifié ou supprimé entre-temps sans effet sur le brouillon.
- **Contrôle de taille.** À l'ajout, le noyau compare la taille totale à la limite annoncée par le serveur SMTP (extension `SIZE`) et avertit si elle est dépassée.
- **Durée de vie.** Un fichier est supprimé quand plus aucun `draft_attachment` ne le référence, c'est-à-dire après l'envoi ou l'abandon du brouillon. Après l'envoi, les pièces jointes vivent dans le brut du mail envoyé, rangé dans Sent. Le ramasse-miettes de `stockage-brut.md` (section 7) traite aussi ce dossier, avec le même verrou.
- **Brouillon créé ailleurs.** Ses pièces jointes sont extraites du brut et copiées dans `outgoing.noindex` à l'ouverture du brouillon.

## 8. Commandes du contrat

| Commande | Effet |
| --- | --- |
| `create_draft(account, reply_to?)` | Crée un brouillon, renvoie son identifiant et son contenu initial |
| `open_draft(id)` | Marque le brouillon ouvert, renvoie sa dernière version |
| `update_draft(id, revision, champs)` | Enregistre une modification. Refusée si `revision` n'est pas la révision courante : deux fenêtres, ou l'app et le CLI, ne s'écrasent pas |
| `add_draft_attachment(id, path)`, `remove_draft_attachment(id, attachment_id)` | Copie le fichier dans la zone privée, ou retire la pièce jointe |
| `close_draft(id)` | Ferme le brouillon et l'envoie au serveur s'il a été modifié |
| `discard_draft(id)` | Abandonne le brouillon |
| `send_draft(id, after_send?)` | Envoie le brouillon (section 9) |

`send_message` reste disponible pour le CLI et l'agent MCP : il crée un brouillon et l'envoie dans la même commande, sans passer par Drafts.

L'événement `DraftChanged {id}` signale une adoption ou un conflit, pour que le client mette à jour son affichage.

## 9. Envoi

1. `send_draft` passe le brouillon à `sending` : il n'est plus modifiable. Le MIME final est construit avec le `Message-ID` stable, sans l'en-tête `X-NiceMailer-Draft-Revision`. L'action `send` est mise en file.
2. Après l'acceptation par le serveur SMTP : copie dans Sent par `APPEND` (sauf chez les fournisseurs qui la font eux-mêmes), suppression de la copie dans Drafts, suppression de la ligne `draft` et de ses pièces jointes, puis rangement de la conversation selon `after_send` (`dossiers.md`, « Répondre à une conversation »).
3. La ligne `message` du brouillon devient celle du mail envoyé : elle passe dans Sent, avec l'UID de la copie (`APPENDUID`) et `is_draft` à 0. Le MIME construit à l'étape 1 est enregistré dans le stockage brut (`stockage-brut.md`) : c'est exactement ce qui a été envoyé, il n'y a pas à le retélécharger. Chez un fournisseur qui crée lui-même la copie dans Sent, la ligne est rattachée à cette copie par son `Message-ID`, comme après un déplacement sans UIDPLUS (`modele de donnees.md`, section 6).

**Double envoi.** Un plantage entre l'acceptation SMTP et le passage de l'action à `done` peut provoquer un second envoi à la reprise. Le `Message-ID` stable permet une parade partielle : avant de renvoyer, le noyau cherche ce `Message-ID` dans Sent. Ce n'est pas une garantie, puisque le plantage peut survenir avant la copie dans Sent. Le traitement complet de l'idempotence de l'envoi reste à spécifier.

## 10. Questions ouvertes

- **Format de rédaction.** Texte brut, HTML, Markdown ? `body_format` le prévoit, mais l'éditeur n'est pas spécifié.
- **`Message-ID` chez les autres clients.** Mail sur iOS, Thunderbird et les webmails conservent-ils le `Message-ID` d'un brouillon qu'ils modifient ? À tester : cela décide si l'adoption est automatique ou si l'utilisateur voit deux brouillons.
- **Fenêtre ouverte longtemps.** Faut-il envoyer quand même une copie au serveur après une longue inactivité (une heure ?), pour qu'une fenêtre oubliée ne prive pas le téléphone du brouillon ?
