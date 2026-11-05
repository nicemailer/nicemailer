# NiceMailer : événements du noyau

Statut : validé. Complète la section 5 de `architecture.md` (contrat entre le noyau et les clients).

## 1. Objet

Les clients (app, TUI, MCP, serveur web) affichent un état obtenu par des requêtes, puis le tiennent à jour grâce aux événements que le noyau leur pousse. Ce document spécifie comment un client sait qu'il n'a rien manqué, et comment il se rattrape dans le cas contraire.

Trois situations font perdre des événements :

1. **Déconnexion.** Le noyau redémarre (mise à jour, plantage) : les événements émis pendant la coupure ne sont reçus par personne.
2. **Course entre requête et abonnement.** Un changement survenu entre la lecture de la base et l'abonnement est perdu ; à l'inverse, un événement peut concerner un état que la requête intègre déjà.
3. **Client lent.** Un client qui ne lit plus ses événements (MCP inactif, TUI suspendue) ne doit ni faire grossir la mémoire du noyau sans limite, ni bloquer le moteur.

Le CLI en commandes ponctuelles ne s'abonne pas et n'est pas concerné.

## 2. Principes

- **Les événements sont des invalidations.** Ils signalent ce qui a changé (un dossier, une conversation, un message, un compte), sans porter l'état complet. Le client recharge ce qu'il affiche. Une requête locale coûte une fraction de milliseconde, et un client n'affiche que des fenêtres paginées : recharger est gratuit.
- **Un doublon est sans effet, une perte se rattrape.** Recevoir deux fois une invalidation provoque un rechargement inutile, jamais un état faux. Le seul problème à résoudre est de détecter une perte.
- **Le rechargement complet est le filet de sécurité unique.** Tout client abonné doit savoir recharger toutes ses vues à tout moment. Redémarrage du noyau, trou détecté et débordement aboutissent tous à ce rechargement.
- **Rien en base.** Le mécanisme vit en mémoire dans le noyau ; le schéma SQLite n'est pas touché.

## 3. Époque et séquence

Le noyau tient :

- une **époque** (`epoch`) : identifiant aléatoire tiré à chaque démarrage du noyau ;
- une **séquence** (`seq`) : entier incrémenté à chaque événement séquencé, remis à 0 à chaque nouvelle époque.

Un événement séquencé reçoit son `seq` **après la validation** de la transaction SQLite qui a produit le changement. Tout changement dont le `seq` est connu est donc déjà visible en base.

Le noyau garde les `events.replay_buffer` derniers événements séquencés (10 000 par défaut) dans un tampon circulaire, pour pouvoir les rejouer.

## 4. Abonnement et rejeu

`subscribe(since?)` abonne le client aux événements. `since`, facultatif, est la dernière position connue du client : `{epoch, seq}`.

```json
→ {"jsonrpc":"2.0","id":7,"method":"subscribe","params":{"since":{"epoch":"e_7c1","seq":4211}}}
← {"jsonrpc":"2.0","id":7,"result":{"epoch":"e_7c1","seq":4218,"replayed":true}}
← {"jsonrpc":"2.0","method":"ThreadChanged","params":{"epoch":"e_7c1","seq":4212,"thread_id":"t_913","folder_ids":["f_1"]}}
   … événements 4213 à 4218, puis le flux continu
```

- **Sans `since`** (premier abonnement) : le noyau répond avec la position courante ; aucun rejeu.
- **`since` dans l'époque courante et encore couvert par le tampon** : le noyau rejoue les événements de `since.seq + 1` à la position courante, puis continue le flux. `replayed` vaut `true`.
- **Sinon** (époque différente parce que le noyau a redémarré, ou `since.seq` sorti du tampon) : le noyau envoie `ResyncRequired` avec la position courante, puis continue le flux. Le client recharge toutes ses vues.

## 5. Requêtes : position de lecture

Chaque réponse de requête porte `as_of: {epoch, seq}`, la position de la séquence au moment de la lecture.

```json
→ {"jsonrpc":"2.0","id":8,"method":"list_threads","params":{"folder":"inbox","limit":50,"cursor":null}}
← {"jsonrpc":"2.0","id":8,"result":{"items":[ /* ThreadSummary… */ ],"next_cursor":"c_8f2a","as_of":{"epoch":"e_7c1","seq":4218}}}
```

Le noyau lit `as_of` **avant** d'ouvrir la transaction de lecture. Un changement validé après la lecture de `as_of` mais avant l'ouverture de la transaction est à la fois dans la réponse et dans un événement de `seq` supérieur : c'est un doublon, inoffensif. L'inverse (un changement absent de la réponse et d'un événement de `seq` inférieur ou égal) est impossible, puisque le `seq` est attribué après la validation (section 3).

## 6. Clients lents

Chaque client abonné a une file d'événements bornée (`events.client_queue`, 1 000 par défaut). Si elle déborde, le noyau la vide et y place un seul `ResyncRequired`. Le moteur n'attend jamais un client.

## 7. Procédure côté client

1. Se connecter, puis s'abonner, avec `since` si le client a déjà une position (reconnexion).
2. Faire les requêtes des vues à afficher, en notant leur `as_of`.
3. Pour chaque événement reçu :
   - `ResyncRequired` : recharger toutes les vues ;
   - époque différente de celle du dernier événement, ou `seq` non consécutif au précédent : trou détecté, recharger toutes les vues ;
   - sinon, si `seq` est supérieur au `as_of` de la vue concernée : recharger les éléments invalidés qu'elle affiche ; sinon, ignorer l'événement.
4. Mémoriser la position du dernier événement reçu, pour la prochaine reconnexion.

L'abonnement précède les requêtes : un événement émis pendant une requête est ainsi reçu, et `as_of` dit s'il faut l'appliquer.

## 8. Catalogue des événements

Les événements **séquencés** passent par le tampon et sont rejoués. Les événements **transitoires** sont envoyés aux clients connectés et jamais rejoués : leur perte est sans conséquence.

| Événement | Paramètres | Type | Signification |
| --- | --- | --- | --- |
| `FolderChanged` | `folder_id` | séquencé | Contenu ou compteurs du dossier modifiés |
| `ThreadChanged` | `thread_id`, `folder_ids` | séquencé | Conversation créée, modifiée ou supprimée, dans les dossiers indiqués |
| `MessageChanged` | `message_id`, `thread_id` | séquencé | Message modifié (flags, dossier, corps réinterprété, consultation) ou supprimé |
| `MessageArrived` | `message_id`, `folder_id` | séquencé | Nouveau mail, trié dans le dossier indiqué ; sert aussi aux notifications. Émis après le `FolderChanged` du dossier, et jamais pendant la première synchronisation d'un dossier, dont tous les mails seraient « nouveaux » |
| `AccountChanged` | `account_id` | séquencé | Compte ajouté, modifié ou supprimé |
| `AccountError` | `account_id`, `error` | séquencé | Erreur de connexion ou d'authentification d'un compte |
| `ActionReverted` | `message_id`, `kind`, `reason` | séquencé | Action locale annulée après un échec définitif (`modele de donnees.md`, section 6) |
| `SendConfirmationRequested` | `request_id` | séquencé | Un agent demande un envoi (`architecture.md`, section 9) |
| `DraftChanged` | `draft_id` | séquencé | Brouillon remplacé par une version modifiée ailleurs, ou dédoublé après un conflit (`brouillons.md`, section 6) |
| `ConfigChanged` | `keys` | séquencé | Réglages de `config.toml` modifiés, par une commande ou une édition directe (`configuration.md`). `keys` vide : le fichier est de nouveau valide après un `ConfigError`, sans changement des réglages appliqués |
| `ConfigError` | `line`, `key`, `message` | séquencé | `config.toml` invalide : la configuration précédente reste appliquée. `line` (à partir de 1) et `key` (clé pointée) valent `null` quand ils ne s'appliquent pas (fichier illisible) ; l'état reste disponible par `config_status` |
| `ResyncRequired` | `epoch`, `seq` | séquencé | Le client doit recharger toutes ses vues |
| `SyncProgress` | `account_id`, `folder_id` (`null` pour les corps), `stage` (`headers` ou `bodies`), `done`, `total` | transitoire | Avancement d'une synchronisation, quatre par seconde au plus. Sans `seq` : il ne prend aucune place dans la séquence, et un client ne doit pas y voir un trou |
| `ImportProgress` | `job_id`, `path`, `account_id`, `running`, `bytes_read`, `total_bytes`, `imported`, `skipped`, `parse_errors` | séquencé | Avancement d'un import mbox, un par lot de mails écrits (`import-initial.md`, section 9) |
| `ImportFinished` | mêmes paramètres, plus `sent`, `archived`, `correspondents`, `seconds`, et `error` en cas d'échec | séquencé | Fin d'un import mbox, réussi ou non ; l'état reste disponible par `import_status` |

`ImportProgress` est séquencé, contrairement à `SyncProgress` : il n'en part qu'un par lot de 500 mails (quelques-uns par seconde), et un événement transitoire ne porte pas de numéro de séquence, ce qui ferait croire à un trou aux clients qui vérifient la continuité. Pendant un import, chaque lot émet aussi `FolderChanged` pour les dossiers remplis, mais pas de `ThreadChanged` par conversation : des milliers d'événements par seconde satureraient le tampon de rejeu pour une information que `FolderChanged` suffit à transmettre (recharger la liste).

**Un état ne repose jamais sur un événement seul.** Ce qu'un événement signale doit aussi pouvoir être obtenu par une requête, pour qu'un rechargement complet le retrouve. Par exemple, les demandes d'envoi en attente de confirmation sont listées par `list_pending_confirmations`, et l'état des comptes par `list_accounts`.

Après un `ResyncRequired`, les `MessageArrived` perdus ne produisent pas de notification macOS : c'est une limite assumée.

## 9. Paramètres

Ces paramètres sont des clés de `config.toml` ; le catalogue complet est dans `configuration.md`, section 11.

| Paramètre | Rôle | Valeur initiale |
| --- | --- | --- |
| `events.replay_buffer` | Nombre d'événements séquencés gardés pour le rejeu | 10 000 |
| `events.client_queue` | Taille maximale de la file d'un client | 1 000 |

## 10. Évolution possible

Si un client gardant un gros cache local apparaît (interface web hors ligne, client distant), un rechargement complet après chaque redémarrage du noyau pourrait devenir coûteux. On passerait alors à un journal de changements persistant, à la manière de JMAP (`/changes`) : un compteur stocké en base, un numéro de changement sur les lignes modifiées, une table des suppressions. Ce n'est pas justifié pour des clients locaux qui n'affichent que des fenêtres paginées.

## 11. Tests exigés

- Reconnexion dans la même époque, position couverte par le tampon : rejeu exact, sans trou ni rechargement.
- Reconnexion après redémarrage du noyau : `ResyncRequired`.
- Position sortie du tampon : `ResyncRequired`.
- Changement validé pendant une requête : présent dans la réponse ou dans un événement de `seq` supérieur à `as_of`, jamais absent des deux.
- Client qui ne lit plus : la file déborde, un seul `ResyncRequired` est envoyé, le moteur continue sans attendre.
- Trou simulé dans la séquence côté client : rechargement complet.
