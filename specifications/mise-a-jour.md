# NiceMailer : mise à jour et migration

Statut : validé. Complète la section 6 de `architecture.md` (processus et packaging).

## 1. Objet et principe

Ce document spécifie le déroulement d'une mise à jour de NiceMailer par Sparkle, et de la migration éventuelle de la base qui l'accompagne.

**Principe : pendant une mise à jour ou une migration, le noyau ne sert aucun client.** Aucune requête ni commande n'est exécutée, quel que soit le client (app, CLI, TUI, MCP, serveur web).

## 2. Le mode maintenance

Pendant une mise à jour ou une migration, le noyau est en **mode maintenance**.

- Le noyau **reste à l'écoute** sur son socket, mais répond à toute poignée de main par l'erreur `Maintenance`, puis ferme la connexion. L'erreur porte la raison (`update`, `migration`, `newer_schema`, `migration_failed`) et, pour une migration, son avancement.
- Les connexions déjà ouvertes reçoivent la même erreur et sont fermées.

**Pourquoi ne pas simplement refuser les connexions.** Un client qui ne peut pas se connecter applique la procédure de démarrage à la demande (`architecture.md`, section 6) : il lance `launchctl kickstart`, puis abandonne au bout de 3 s avec un message trompeur. Pendant le remplacement du bundle, ce `kickstart` pourrait même lancer un binaire à moitié remplacé. Avec une réponse `Maintenance` explicite, le client sait qu'il doit attendre, et ne tente aucun démarrage.

## 3. Déroulement d'une mise à jour

1. **Sparkle trouve une mise à jour** et l'utilisateur l'accepte (ou elle s'installe automatiquement, selon ses réglages).
2. **Avant l'installation**, l'app envoie la commande `prepare_update` au noyau. Le noyau :
   - passe en mode maintenance (raison `update`) ;
   - termine les transactions en cours et ferme les connexions IMAP ; les actions du spool restent en file (`queued`) ;
   - **reste en vie** en mode maintenance. S'il s'arrêtait, un client qui se connecte relancerait l'ancien binaire par `kickstart`.
3. **Sparkle remplace le bundle** et relance l'app.
4. **L'app exécute `launchctl kickstart -k`** : l'ancien noyau reçoit SIGTERM et s'arrête, le nouveau démarre.
5. **Le nouveau noyau** prend le verrou d'instance, se met à l'écoute, applique les migrations nécessaires (section 4) en mode maintenance (raison `migration`), puis sort du mode maintenance.
6. **Les clients se reconnectent.** L'époque a changé : leur abonnement reçoit `ResyncRequired` et ils rechargent leurs vues (`evenements.md`).

**Mise à jour abandonnée** (échec du téléchargement, refus de l'utilisateur, plantage de l'app) : l'app envoie `cancel_update` si elle le peut. Sinon, le noyau sort de lui-même du mode maintenance après `core.update_timeout_secs` (900 s par défaut).

`prepare_update` et `cancel_update` ne sont acceptées que de l'app (vérification de signature, `architecture.md`, section 9).

## 4. Migration de la base

Au démarrage, le noyau compare `PRAGMA user_version` à la version de schéma qu'il attend.

| Situation | Comportement |
| --- | --- |
| Version égale | Aucune migration |
| Version inférieure | Migration, en mode maintenance (raison `migration`) |
| Version supérieure (retour à une ancienne version de NiceMailer) | Mode maintenance permanent, raison `newer_schema` : le noyau ne touche pas à une base qu'il ne comprend pas |

Déroulement d'une migration :

1. Sauvegarder la base par l'API de sauvegarde de SQLite dans `nicemailer.db.bak-v<ancienne version>`. Les fichiers bruts ne sont pas concernés : leur format ne dépend pas du schéma.
2. Appliquer les migrations manquantes dans l'ordre, chacune dans une transaction, en mettant `user_version` à jour dans la même transaction.
3. En cas d'échec, restaurer la sauvegarde et rester en mode maintenance avec la raison `migration_failed`. L'app affiche l'erreur et propose d'exporter un rapport ; elle ne relance pas la migration d'elle-même.
4. En cas de succès, garder la sauvegarde jusqu'à la mise à jour suivante, puis la supprimer.

Une migration longue (grosse base) publie son avancement dans la réponse `Maintenance`.

## 5. Comportement des clients

| Client | Réponse `Maintenance` |
| --- | --- |
| App | Bandeau « Mise à jour de NiceMailer… » ou « Mise à jour de la base… », avec l'avancement ; nouvelle tentative de connexion toutes les secondes. Pour `newer_schema` et `migration_failed`, message d'erreur avec la marche à suivre. |
| CLI | Message explicite et code de sortie dédié. Avec l'option `--wait`, attend la fin de la maintenance. |
| TUI, MCP | Message d'attente, nouvelle tentative toutes les secondes. |

## 6. Paramètres

| Paramètre | Rôle | Valeur initiale |
| --- | --- | --- |
| `core.update_timeout_secs` | Durée maximale du mode maintenance pour une mise à jour sans suite | 900 |

## 7. Tests exigés

- Mise à jour complète : aucun client servi entre `prepare_update` et la fin de la migration ; les clients se reconnectent et reçoivent `ResyncRequired`.
- Client qui se connecte pendant la maintenance : erreur `Maintenance`, aucun `kickstart`.
- Mise à jour abandonnée : sortie de maintenance par `cancel_update`, puis par expiration du délai.
- Migration qui échoue à mi-parcours : base restaurée, raison `migration_failed`.
- Base d'une version plus récente que le noyau : `newer_schema`, aucune écriture.
- Actions du spool en file avant la mise à jour : exécutées après, sans perte ni doublon.
