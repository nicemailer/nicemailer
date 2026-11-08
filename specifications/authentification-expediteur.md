# NiceMailer : authentification de l'expéditeur

Statut : validé pour la v1. Complète `dossiers.md` (screening), `schema.sql` et `modele de donnees.md`.

## 1. Objet

Le screening (voir `dossiers.md`) classe les mails entrants selon leur expéditeur, lu dans l'en-tête `From`. Cet en-tête est librement falsifiable : sans contrôle, un attaquant qui se fait passer pour un expéditeur approuvé arrive directement dans l'Inbox, ce qui est l'inverse de ce qu'on attend d'un screener.

Ce document spécifie comment le noyau détermine, pour chaque mail reçu, si l'expéditeur annoncé est authentifié, et comment ce verdict conditionne l'application des règles de screening.

## 2. Principes

- **Une règle restrictive s'applique toujours.** Un mail qui se réclame d'un expéditeur banni va dans Banned, authentifié ou non : au pire, un mail usurpé part dans Banned au lieu de Quarantine.
- **Une règle favorable** (Inbox, Newsletter, Paperwork) **exige un niveau d'authentification suffisant.** Sinon, le mail va en Quarantine avec un avertissement.
- **Une règle de domaine favorable exige toujours un mail authentifié.** Approuver tout un domaine sans authentification ouvrirait la porte à l'usurpation du domaine entier.
- **Confiance au premier usage.** On exige d'un expéditeur le niveau d'authentification qu'il a déjà montré : un expéditeur déjà vu authentifié ne peut plus être accepté sans l'être.
- **Dégradation douce.** Quand l'information manque (serveur qui ne fournit pas de verdict), le comportement retombe sur celui d'un screening sans authentification, signalé par un badge, et jamais sur un blocage général.
- **Le verdict est un fait daté.** Il est calculé une fois, à la réception, et n'est pas recalculé ensuite.

## 3. Le verdict d'authentification

Chaque message reçu porte un verdict (`message.auth_result`) :

| Verdict | Signification |
|---|---|
| `pass` | DMARC réussi, ou signature DKIM valide et alignée sur le domaine du `From` |
| `fail` | DMARC échoué |
| `none` | Le domaine ne publie pas de politique DMARC et aucune signature DKIM alignée n'est valide : impossible d'authentifier |
| `unknown` | Pas d'information fiable : aucun en-tête de confiance, erreur temporaire du serveur (`temperror`), `From` absent ou multiple |

`message.auth_source` indique d'où vient le verdict : `server` (en-tête `Authentication-Results`) ou `local` (vérification par le noyau, hors v1). Il est NULL quand le verdict est `unknown` faute de source.

### Source v1 : `Authentication-Results` du serveur

Le serveur de réception vérifie SPF, DKIM et DMARC et écrit son verdict dans un en-tête `Authentication-Results` (RFC 8601), placé en tête du message. En v1, c'est la seule source.

**Choix de l'en-tête de confiance.** Un expéditeur malveillant peut inclure lui-même un faux `Authentication-Results: dmarc=pass`. Le noyau ne retient donc que :

1. les en-têtes dont l'identifiant de serveur (`authserv-id`) est égal à l'`authserv_id` du compte : la clé `authserv_id` du compte dans `accounts.toml` si elle est présente, sinon `account.authserv_id` détecté ;
2. parmi eux, le premier en partant du haut du message, c'est-à-dire le dernier ajouté.

Un serveur conforme supprime à la réception les en-têtes qui portent déjà son identifiant (RFC 8601, section 5). Le premier en-tête retenu est donc le sien.

Si le compte n'a pas d'`authserv_id` (ni dans `accounts.toml`, ni détecté), aucun en-tête n'est retenu et le verdict est `unknown`.

**Traduction en verdict**, à partir des méthodes de l'en-tête retenu :

1. `dmarc=pass` → `pass`.
2. `dmarc=fail` → `fail`.
3. `dmarc=temperror` ou `dmarc=permerror` → `unknown`.
4. Pas de résultat DMARC, ou `dmarc=none` :
   - `dkim=pass` avec un `header.d` aligné sur le domaine du `From` → `pass` ;
   - sinon → `none`.

L'alignement est l'alignement relâché de DMARC : même domaine organisationnel, déterminé avec la Public Suffix List embarquée (`mail.exemple.fr` est aligné sur `exemple.fr`).

### Détection de `authserv_id`

À l'ajout d'un compte, le noyau examine le premier en-tête `Authentication-Results` des 50 derniers mails de l'INBOX. Si un même `authserv-id` apparaît en tête dans au moins 90 % d'entre eux, il est enregistré dans `account.authserv_id`. Sinon, la colonne reste NULL.

Tant qu'elle est NULL, la détection est refaite périodiquement sur les nouveaux mails, au cas où le serveur se mettrait à ajouter l'en-tête : à une passe de synchronisation qui apporte des mails à l'Inbox, sur ses 50 UID les plus hauts, au plus une fois par heure (phase 6 de `plan-imap.md`). La valeur détectée est visible dans les réglages avancés du compte. La modifier écrit la clé `authserv_id` du compte dans `accounts.toml`, qui prend alors le pas sur la détection (`configuration.md`, section 6).

### Moment du calcul

Le verdict est calculé à la synchronisation des en-têtes (`body_state = 0`), dans la même transaction que l'insertion du message et avant le screening. La récupération des en-têtes doit donc inclure `Authentication-Results`. Le screening reste instantané : il n'attend pas le téléchargement du message complet.

Le verdict est calculé pour tous les messages reçus, quel que soit le dossier, pour pouvoir afficher le badge « non vérifié » partout.

## 4. Application au screening

### Politique d'authentification par adresse

Deux colonnes de `address` portent la confiance au premier usage :

- `auth_seen` : 1 dès qu'un mail `pass` a été reçu de cette adresse, sur n'importe quel compte. Il n'est jamais remis à 0 par l'ingestion.
- `auth_policy` : choix de l'utilisateur pour cette adresse.

| `auth_policy`   | Verdicts acceptés pour une règle favorable                |
| --------------- | --------------------------------------------------------- |
| `auto` (défaut) | `pass` ; `none` et `unknown` seulement si `auth_seen = 0` |
| `required`      | `pass` seulement                                          |
| `not_required`  | tous, y compris `fail`                                    |

`not_required` sert d'issue de secours pour un expéditeur légitime qui envoie depuis un outil mal configuré (CRM, ticketing, invitation d'agenda) dont les mails échouent à DMARC. `required` sert aux expéditeurs sensibles (banque, administration), pour bloquer tout mail non authentifié même avant qu'un mail authentifié ait été vu.

### Algorithme

Pour un mail entrant dans l'INBOX, dont l'expéditeur est l'adresse `A`, de domaine `D` :

1. Si `A.screening_target = 'banned'`, ou si `A` n'a pas de règle et `D.screening_target = 'banned'` : **Banned**.
2. Si `A` a une règle favorable :
   - si le verdict est accepté par `A.auth_policy` (tableau ci-dessus) : **cible de la règle** ;
   - sinon : **Quarantine**, avec `message.auth_suspect = 1`.
3. Sinon, si `D` a une règle favorable :
   - si le verdict est `pass` : **cible de la règle** ;
   - sinon : **Quarantine**, avec `message.auth_suspect = 1`.
4. Sinon, si le mail répond à une conversation connue (`dossiers.md`, « Réponse dans une conversation connue ») :
   - si le verdict n'est pas `fail` : **Inbox**, sans décision sur l'adresse ;
   - sinon : **Quarantine**, avec `message.auth_suspect = 1`.
5. Sinon : **Quarantine** (expéditeur inconnu).

Dans la même transaction, si le verdict est `pass`, `A.auth_seen` passe à 1.

Un mail sans `From`, ou avec plusieurs adresses dans `From`, a le verdict `unknown` et va en Quarantine : aucune règle d'adresse ne peut lui être appliquée.

### Exemples

| Situation | Verdict | Résultat |
|---|---|---|
| Un attaquant usurpe `support@banque.fr` (DMARC en `p=reject`) | `fail` | Quarantine, avertissement |
| Un attaquant usurpe `jean@gmail.com` | `fail` | Quarantine, avertissement |
| Un attaquant usurpe `contact@pme.fr`, domaine sans DKIM ni DMARC, jamais vu authentifié | `none` | Inbox, badge « non vérifié » (**non couvert**) |
| Vrai mail de `contact@pme.fr` | `none` | Inbox, badge « non vérifié » |
| Collègue `@entreprise.fr`, déjà vu authentifié, qui envoie depuis un CRM non signé | `fail` ou `none` | Quarantine, avertissement ; l'utilisateur peut passer l'adresse en `not_required` |
| Compte dont le serveur ne fournit pas d'en-tête de confiance | `unknown` | Règles appliquées comme sans authentification, badge « non vérifié » |

## 5. Interface

- **Badge « non vérifié »** sur tout mail accepté dont le verdict n'est pas `pass`.
- **Avertissement « usurpation probable »** sur tout mail en Quarantine avec `auth_suspect = 1`, avec le verdict et le domaine concernés.
- **Adresse complète affichée** dans la Quarantine, pas seulement le nom d'affichage : `PayPal <x@evil.com>` doit être lisible comme tel. Les usurpations par nom affiché et par domaine ressemblant (`paypa1.com`) ne correspondent à aucune règle et arrivent déjà en Quarantine ; c'est l'utilisateur qui doit éviter de les approuver.
- **Sortie de Quarantine d'un mail suspect** : l'app demande une confirmation explicite et propose de choisir `auth_policy` pour l'adresse (`auto` par défaut, `not_required` pour un outil mal configuré). Le noyau refuse la décision favorable sans confirmation (erreur `1005`, `CONFIRMATION_REQUIRED`, avec l'adresse et le verdict) ; un déplacement vers Banned d'un mail suspect ne concerne que ce mail, sans décision.
- **Réglage par adresse** : `auth_policy` est modifiable depuis la fiche du contact.

## 6. Données

```sql
-- account
authserv_id  TEXT,      -- authserv-id détecté ; NULL = aucun détecté
                        -- (la clé authserv_id d'accounts.toml le remplace)

-- message
auth_result  TEXT NOT NULL DEFAULT 'unknown'
             CHECK (auth_result IN ('pass','fail','none','unknown')),
auth_source  TEXT CHECK (auth_source IN ('server','local')),
auth_suspect INTEGER NOT NULL DEFAULT 0 CHECK (auth_suspect IN (0,1)),

-- address
auth_seen    INTEGER NOT NULL DEFAULT 0 CHECK (auth_seen IN (0,1)),
auth_policy  TEXT NOT NULL DEFAULT 'auto'
             CHECK (auth_policy IN ('auto','required','not_required')),
```

`domain` n'a pas de politique : une règle de domaine favorable exige toujours `pass`.

`reindex` (voir `stockage-brut.md`) ne recalcule ni `auth_result` ni `auth_suspect`. Le verdict a servi à une décision de screening déjà prise, et une vérification refaite plus tard peut différer (changement de `authserv_id`, ou clés DKIM renouvelées pour une future vérification locale).

## 7. Hors périmètre v1

- **Vérification DKIM locale** (source `local`). Elle couvrirait les serveurs qui ne fournissent pas d'en-tête, mais exige le message complet avant le screening, dépend du DNS et produit de faux échecs quand le serveur modifie le corps (mention antivirus, réécriture des liens). Elle sera décidée sur mesure : la commande `nicemailer diag auth` indique, par compte, la proportion de mails reçus avec un en-tête de confiance. La crate `mail-auth` est la candidate.
- **SPF côté client** : impossible sans l'IP de connexion, que seul le serveur connaît.
- **ARC** : un résultat ARC valide pourrait sauver les mails de listes de diffusion dont DKIM est cassé. Non pris en compte en v1 ; ces mails sont traités comme `fail` et l'utilisateur peut utiliser `not_required`.

## 8. Tests exigés

- Faux `Authentication-Results: dmarc=pass` inséré par l'expéditeur, sous l'en-tête du serveur : ignoré, verdict tiré de l'en-tête du serveur.
- Faux en-tête avec un autre `authserv-id`, sans en-tête du serveur : verdict `unknown`.
- Chaque ligne du tableau de traduction de la section 3, y compris l'alignement par domaine organisationnel (`mail.exemple.fr` / `exemple.fr`, et un suffixe public comme `co.uk`).
- Chaque ligne du tableau des exemples de la section 4.
- `auth_seen` passe à 1 au premier `pass` et n'est jamais remis à 0.
- `From` absent, `From` multiple : `unknown`, Quarantine.
- Détection de `authserv_id` : seuil de 90 % atteint, non atteint, aucun en-tête.
- `reindex` complet : `auth_result` et `auth_suspect` inchangés.
- Réponse d'un expéditeur sans décision dans une conversation connue : Inbox, aucune décision enregistrée ; avec un verdict `fail` : Quarantine, `auth_suspect = 1`.
- Réponse à une conversation qui n'existe qu'en Newsletter ou en Quarantine : Quarantine.
- Expéditeur banni répondant dans une conversation connue : Banned.

## 9. Questions ouvertes

1. **Microsoft.** Sauf erreur, Microsoft 365 et Outlook.com omettent l'`authserv-id` dans `Authentication-Results`, contrairement à la RFC. À vérifier sur de vrais mails ; si c'est confirmé, il faut une règle propre au fournisseur pour reconnaître l'en-tête du serveur.
2. **Serveurs qui ne suppriment pas les en-têtes entrants** portant leur identifiant. Le choix du premier en-tête protège tant que le serveur ajoute le sien ; à vérifier sur les hébergeurs ciblés.
3. **Seuils de détection** (50 mails, 90 %) : à ajuster sur des comptes réels.
