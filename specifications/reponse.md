# NiceMailer : répondre à un mail (v1)

Statut : validé pour la v1 (7 novembre 2026). Cette spécification décrit la première version de la réponse. Elle s'appuie sur `brouillons.md` (cycle de vie d'un brouillon, envoi), `dossiers.md` (« Répondre à une conversation », « Écrire à quelqu'un vaut décision Inbox ») et `ui.md` (contexte « edition »). Quand elle restreint ces documents, c'est le périmètre de la v1 qui est restreint, pas leurs règles : les parties écartées restent à faire.

## 1. Scénario

1. L'utilisateur lit un mail.
2. Il tape `r`. La réponse est adressée à tout le monde (équivalent de « répondre à tous »).
3. Une zone de saisie en texte seul remplace le lecteur. Aucune mise en forme n'est possible.
4. Le mail auquel il répond est cité sous la zone de saisie, de façon classique.
5. `⌘↩` envoie le mail.
6. La conversation est rangée selon les règles du workflow des dossiers.

## 2. Périmètre

| Dans la v1 | Plus tard |
| --- | --- |
| Réponse à tous (`r`) | Réponse à l'expéditeur seul (`R`, réservé), transfert (`f`), nouveau mail |
| Texte seul, sans mise en forme | Mise en forme, HTML |
| Brouillon enregistré en local (table `draft`) | Copie dans Drafts sur le serveur, `append_draft`, conflits entre appareils (`brouillons.md`, sections 5 et 6) |
| Destinataires calculés par le noyau, affichés | Lignes À, Cc et Cci éditables |
| Rangement présélectionné par le noyau, affiché ; choix manuel à l'envoi par `⇧⌘↩` (ajouté en novembre 2026, section 8) | |
| Envoi par le spool, délai d'annulation de 5 s | Pièces jointes, signature |
| Copie dans Sent | Commande CLI `nicemailer reply` |
| Écrire à quelqu'un vaut décision Inbox | |

## 3. Le mail auquel on répond

`r` répond au **mail sous le curseur** du lecteur. À l'ouverture d'une conversation, c'est le dernier mail reçu, pas un mail de l'utilisateur. Dans la liste, sans lecteur ouvert, `r` répond au mail que le lecteur aurait mis en évidence.

`r` ne fait rien (message dans la barre d'état) quand :

- le compte n'a pas de serveur SMTP dans `accounts.toml`, ou est en lecture seule (`read_only`) ;
- le compte est un compte d'archive (import mbox) ;
- le mail est un brouillon, ou n'a pas encore de corps téléchargé (le corps est demandé en priorité, `r` fonctionne dès qu'il est arrivé).

**Brouillon existant.** S'il existe déjà un brouillon de réponse au même mail, `r` le rouvre au lieu d'en créer un second.

## 4. Préparation de la réponse (noyau)

Tout est calculé par le noyau (`create_draft(account, reply_to)`), conformément au principe « aucune logique métier côté Swift ». Les règles pures vivent dans `nm-core`.

### Adresse d'expédition

La première des identités du compte (adresse principale, puis identités d'`accounts.toml`, `configuration.md`, section 6) qui figure parmi les destinataires du mail (To, puis Cc ; `Delivered-To` n'est pas conservé en base). À défaut, l'adresse principale du compte. Le nom affiché est celui de l'identité, ou à défaut celui du compte.

### Destinataires (réponse à tous)

1. **À** : les adresses de `Reply-To` si l'en-tête est présent, sinon l'expéditeur (`From`).
2. **Cc** : les destinataires du mail (To puis Cc).
3. **Réponse à son propre mail.** Si l'expéditeur est une adresse du compte, la réponse reprend les destinataires du mail : À reçoit son To, Cc son Cc. C'est une relance, pas une réponse à soi-même.
4. Sont retirées : les adresses du compte (adresse principale et identités), les doublons (comparaison sans tenir compte de la casse ; une adresse présente dans À n'est pas répétée dans Cc), les adresses invalides.
5. Si À est vide après ce filtrage (mail envoyé uniquement à soi-même), la première adresse de Cc passe dans À. S'il ne reste personne, la réponse est adressée à l'expéditeur, même si c'est une adresse du compte.

Les noms affichés sont repris des en-têtes. Aucune adresse n'est ajoutée en Cci.

### Objet

L'objet du mail, précédé de `Re: `. Le préfixe n'est pas ajouté si l'objet commence déjà par un préfixe de réponse (`Re:`, `RE:`, `re:`, `Aw:`, `AW:`, `Sv:`, `Réf. :`…, suivis ou non d'un numéro comme `Re[2]:`) : on ne les empile jamais. Un objet vide donne `Re: `.

### En-têtes de conversation

- `In-Reply-To` : le `Message-ID` du mail auquel on répond.
- `References` : les `References` de ce mail (ou, à défaut, son `In-Reply-To`), suivies de son `Message-ID`. La liste est bornée aux 20 derniers identifiants, en gardant toujours le premier (la racine de la conversation).
- Un mail sans `Message-ID` donne une réponse sans `In-Reply-To` : elle ne range alors rien (`dossiers.md`, « Répondre à une conversation »). Elle reste liée à la conversation localement par `reply_to_message_id`.

### Corps et citation

Le corps initial est : une ligne vide (où se place le curseur), une ligne vide, la ligne d'introduction, puis le texte cité. La réponse est donc **au-dessus de la citation**.

- **Ligne d'introduction**, dans la langue de l'app (le client la passe au noyau) : « Le 7 nov. 2026 à 10:12, Alice Martin <alice@exemple.fr> a écrit : », en anglais « On Nov 7, 2026, at 10:12, Alice Martin <alice@exemple.fr> wrote: ». La date est celle du mail, dans le fuseau du Mac.
- **Texte cité** : la partie `text/plain` du mail ; à défaut, le texte que le lecteur tire du HTML. Chaque ligne est précédée de `> ` ; une ligne déjà citée reçoit `>` sans espace (`>> `). Les lignes vides deviennent `>`. Les espaces de fin de ligne sont retirés.
- **Limite** : au-delà de 1 000 lignes, la citation est tronquée et se termine par `> […]`. Une citation énorme ralentirait l'éditeur sans servir personne.

La citation fait partie du texte éditable : l'utilisateur peut la raccourcir ou écrire entre les lignes citées.

## 5. Brouillon (v1)

La v1 applique la section 4 de `brouillons.md` sans le serveur :

- **Création** : `create_draft` crée la ligne `draft` avec un `Message-ID` neuf, `reply_to_message_id`, l'adresse d'expédition, les destinataires, l'objet et le corps initial.
- **Rédaction** : le client envoie le corps (`update_draft`) à chaque pause de frappe (0,5 s sans frappe), et à la sortie de la zone de saisie. La révision protège contre l'écrasement (`brouillons.md`, section 8).
- **Échap** : ramène au mail, et garde le brouillon. Le prochain `r` sur ce mail le rouvre (section 3).
- **Abandon** : `⌘⌫` dans la zone de saisie abandonne le brouillon (`discard_draft`) et ramène au mail. `z` le restaure, avec son texte, tant que l'app n'a pas été quittée : l'app garde le texte abandonné et recrée le brouillon (`create_draft` avec `body`), qui reçoit un nouveau `Message-ID`.
- **Plantage** : le brouillon est en base à la dernière pause de frappe ; rien n'est perdu au-delà de 0,5 s de frappe.

**Fermeture : le dossier Drafts** (ajouté après la v1, novembre 2026). Échap ferme le brouillon (`close_draft`) : il devient une ligne du dossier Drafts, complète, dans la conversation du mail auquel il répond, et une action `append_draft` le copie dans le dossier Drafts du serveur (`brouillons.md`, sections 4 et 5). Le noyau qui redémarre ferme de même les brouillons modifiés depuis leur dernière copie. Dans Drafts, `r` ou Entrée rouvre le brouillon. Envoyé ou abandonné, il quitte Drafts, ici et sur le serveur. Des modifications faites ailleurs (section 6 de `brouillons.md`), seuls les cas simples sont traités : copie supprimée ou envoyée ailleurs. Les brouillons écrits sur le téléphone restent en lecture seule. Le détail est dans `docs/sending.md`, « Drafts ».

## 6. Écran de réponse

La zone de réponse **remplace le lecteur**, dans la même fenêtre. De haut en bas :

1. **De** : adresse d'expédition, non modifiable.
2. **À** et **Cc** : les destinataires calculés, non modifiables, sur une ligne chacun (tronquée avec le nombre de destinataires cachés, « +3 »). Cc est masqué s'il est vide.
3. **Objet**, non modifiable.
4. **Après l'envoi** : la cible du rangement (« Après l'envoi : Archive »), discrète. Absente quand la conversation reste en place (section 8).
5. **La zone de saisie** : texte seul, police à chasse fixe de l'app, retour à la ligne visuel à la largeur de la zone, sans coupure dans le texte. Le curseur est en tête.

La barre d'état affiche le mode `EDT` (contexte « edition » de `ui.md`), et la ligne du titre rappelle les trois raccourcis ci-dessous. Le texte est édité par l'app elle-même, pas par un champ AppKit (`docs/sending.md`) : ↑ et ↓ suivent les lignes affichées, un clic place le curseur.

Raccourcis de la zone de réponse (contexte « edition » de `ui.md`) :

| Touche | Effet |
| --- | --- |
| `⌘↩` | Envoyer, avec le rangement affiché |
| `⇧⌘↩` | Choisir le rangement, puis envoyer (section 8) |
| `Échap` | Revenir au mail, garder le brouillon |
| `⌘⌫` | Abandonner le brouillon |

Les touches d'une lettre (navigation, dossiers) sont du texte tapé dans ce contexte : aucune ne déclenche de commande.

Un envoi refusé (`⌘↩` sans destinataire, compte sans SMTP) laisse la zone ouverte et affiche la raison dans la barre d'état.

## 7. Envoi

### Mise en file et délai d'annulation

`⌘↩` appelle `send_draft(id)`, sans `after_send` : le noyau applique la présélection (section 8).

1. Le noyau enregistre le dernier corps, passe le brouillon à `sending` (il n'est plus modifiable), construit le MIME final et met en file une action `send` dont `date_run_after` est **maintenant + 5 s**.
2. L'app revient aussitôt au mail. La barre d'état affiche « Envoi dans 5 s · z pour annuler ».
3. **Annulation** : `z` pendant le délai appelle `cancel_send(id)`. L'action passe à `cancelled`, le brouillon revient à `editing`, et la zone de réponse se rouvre avec son texte. Après le délai, ou si l'action a déjà commencé, `cancel_send` est refusé (`SEND_ALREADY_STARTED`) : le mail est parti ou en train de partir.
4. **Plantage pendant le délai** : l'action est en base ; au redémarrage, elle part normalement. Un envoi décidé n'est pas perdu.

Le délai est un réglage (`compose.undo_send_seconds`, de 0 à 30, 5 par défaut). À 0, il n'y a pas d'annulation.

### Format du mail

- `text/plain; charset=utf-8; format=flowed` (RFC 3676), en `quoted-printable` si le texte contient autre chose que de l'ASCII ou des lignes de plus de 998 octets, sinon en `7bit`.
- Les lignes sont coupées à 72 caractères **à l'envoi seulement**, par des espaces de fin de ligne (`format=flowed`) : le texte tapé n'est jamais modifié. Les lignes citées (`>`) gardent leur préfixe sur chaque morceau. Une ligne qui se termine par une espace dans le texte d'origine voit cette espace retirée (sinon elle serait lue comme une ligne coupée).
- En-têtes : `From`, `To`, `Cc` (si non vide), `Subject`, `Date` (heure locale, RFC 5322), `Message-ID` (le `Message-ID` stable du brouillon, `<uuid@domaine-de-l-adresse-d-expédition>`), `In-Reply-To`, `References`, `MIME-Version`, `User-Agent: NiceMailer/<version>`. Les noms et l'objet non ASCII sont encodés (RFC 2047).
- Jamais d'en-tête `Bcc` dans le message.

### SMTP

- Le serveur est celui de `[accounts.<clé>.smtp]` (`configuration.md`). Sécurité `tls` (465), `starttls` (587) ou `none` (25, refusé hors `localhost`, comme pour IMAP).
- **Mot de passe** : l'élément `smtp:<clé>` du trousseau s'il existe, lié au serveur SMTP comme l'est celui d'IMAP ; sinon le mot de passe IMAP du compte (`imap:<clé>`), **seulement si le serveur SMTP est le serveur IMAP ou du même domaine organisationnel** (`smtp.gmail.com` et `imap.gmail.com`). Sans cette règle, un `accounts.toml` modifié pour diriger `[smtp]` ailleurs enverrait le mot de passe IMAP à ce serveur. Ailleurs, l'envoi est refusé avec un message qui demande `nicemailer account password <clé> --smtp`.
- **Identifiant** : `username` de la section `smtp`, sinon l'adresse du compte.
- Authentification `AUTH PLAIN` ou `LOGIN`, après TLS seulement.
- Enveloppe : `MAIL FROM` = adresse d'expédition, `RCPT TO` = tous les destinataires (À, Cc, Cci).
- **Taille** : si le serveur annonce `SIZE` et que le message la dépasse, l'envoi échoue définitivement sans être tenté.

### Résultat de l'envoi

| Ce qui arrive | Effet |
| --- | --- |
| Le serveur accepte (`250` après `DATA`) | Suite de l'envoi (ci-dessous), action `done` |
| Erreur temporaire : connexion, délai, réponse `4xx` | Nouvelle tentative avec le délai croissant du spool (`docs/spool.md`) ; le brouillon reste `sending` |
| Erreur définitive : `5xx`, authentification refusée, taille, 8e échec temporaire | Action `reverted`, le brouillon revient à `editing`, événement `SendFailed {draft_id, reason}`. L'app le dit dans la barre d'état ; le prochain `r` sur le mail rouvre le brouillon avec le texte |
| Certains destinataires refusés (`RCPT TO` en `5xx`), d'autres acceptés | Envoi aux destinataires acceptés ; l'événement `SendPartial {draft_id, refused}` liste les autres. Ne pas envoyer du tout pénaliserait les bons destinataires pour une faute de frappe sur un seul |

**Hors ligne** : l'action attend, avec les mêmes délais. Le brouillon reste `sending` et n'est pas modifiable ; `cancel_send` est accepté tant que l'action n'est pas en cours (`queued`).

### Après l'acceptation

Dans cet ordre, chaque étape pouvant être rejouée après un plantage :

1. **Copie dans Sent** (section 9).
2. **Ligne `message`** du mail envoyé, dans le dossier Sent, rattachée à la conversation du mail auquel on répond ; le MIME envoyé est écrit dans le stockage brut (`brouillons.md`, section 9).
3. **Rangement** de la conversation (section 8).
4. **Décision Inbox** pour les destinataires (section 10).
5. **Suppression** de la ligne `draft`.
6. Ligne `send` dans `audit_log`, événement `DraftSent {draft_id, message_id}`.

Les étapes 2 à 5 sont faites dans une seule transaction.

**Double envoi** : avant d'envoyer, une action rejouée après un plantage cherche le `Message-ID` dans Sent. S'il y est, le mail est considéré comme envoyé et seules les étapes 2 à 6 sont faites. La limite décrite dans `brouillons.md` (plantage entre l'acceptation SMTP et la copie dans Sent) demeure.

## 8. Rangement après l'envoi

La règle est celle de `dossiers.md`, « Répondre à une conversation ». Le choix manuel a été ajouté après la v1 (novembre 2026, fin de section). La présélection :

| Dossier de la conversation | Rangement |
| --- | --- |
| Inbox | Archive |
| Inbox, revenue de Waiting (`$NmWasWaiting`) | Waiting |
| Inbox, revenue de Todo (`$NmWasTodo`) | Todo |
| Todo | Archive |
| Waiting | Waiting (rien n'est déplacé) |
| Newsletter, Paperwork, Archive, Quarantine, autres | Reste en place |

- La présélection est calculée par le noyau (`nm-core`) et exposée au client : `ThreadSummary.reply_target`, et dans le résultat de `create_draft` pour l'affichage « Après l'envoi ». Elle est **recalculée à l'envoi** : la conversation a pu changer de dossier pendant la rédaction.
- Le rangement se fait **après l'acceptation SMTP** : un envoi qui échoue ou qui est annulé ne range rien.
- C'est une commande ordinaire de déplacement de conversation (`move_thread`), avec la source du client qui a envoyé, une ligne `move` dans `audit_log` qui cite le mail envoyé, et le retrait des marques `$NmWasTodo` et `$NmWasWaiting`.
- **Annulation** : la barre d'état affiche « Envoyé · rangé dans Archive · z pour annuler ». `z` annule le **rangement**, pas l'envoi (le mail est parti). Le jeton d'annulation est porté par l'événement `DraftSent`.
- Une réponse sans `In-Reply-To` (mail d'origine sans `Message-ID`) ne range rien.

### Choix manuel (`⇧⌘↩`)

- `⌘↩` envoie avec le rangement affiché. `⇧⌘↩` ouvre un petit menu « Envoyer, puis ranger la conversation dans » : `a` Archive, `w` Waiting, `t` Todo, puis, séparée, `s` « la laisser en place (essai) ». La présélection y est marquée. La touche choisie envoie aussitôt ; un clic sur une ligne aussi. Échap revient au texte sans rien envoyer.
- Le menu n'est proposé que si le noyau propose un rangement (`after_send_target` présent : conversation dans Inbox, Todo ou Waiting). Ailleurs, `⇧⌘↩` le dit dans la barre d'état et n'envoie rien.
- **Le choix explicite l'emporte** sur la présélection recalculée à l'envoi. Il ne déplace pourtant jamais une conversation sortie entre-temps d'Inbox, Todo et Waiting (archivée ailleurs, par exemple) : rien n'est rangé, comme pour la présélection.
- Le choix est **enregistré dans le brouillon** (`draft.after_send`) : un envoi annulé par `z` rouvre la réponse avec lui (« Après l'envoi : Waiting »), et `⌘↩` l'applique. Un nouveau `⇧⌘↩` le remplace.
- **Option d'essai « laisser en place »** (`stay`), visible seulement dans ce menu : la réponse part, les destinataires reçoivent leur décision (section 10), mais la conversation n'est pas touchée : aucun déplacement, et les marques `$NmWasTodo` / `$NmWasWaiting` restent (ce n'est pas une commande sur la conversation). Elle sert la réponse qui laisse la conversation à traiter dans l'Inbox. À évaluer à l'usage : à garder, à retirer, ou à intégrer à `dossiers.md`.

## 9. Copie dans Sent

Le mail envoyé est copié dans le dossier de rôle Sent du compte par `APPEND`, avec le flag `\Seen`, **sauf** quand le fournisseur fait lui-même cette copie, ce qui la doublerait :

| Fournisseur | Détection |
| --- | --- |
| Gmail, Google Workspace | Capacité IMAP `X-GM-EXT-1`, ou hôte SMTP `smtp.gmail.com` / `smtp.googlemail.com` |
| Fastmail | Hôte SMTP en `*.fastmail.com` ou `*.messagingengine.com` |

Le réglage du compte `save_sent` (`auto` par défaut, `always`, `never`) force le choix si la détection se trompe (`configuration.md`).

- L'UID de la copie (`APPENDUID`, UIDPLUS) est enregistré sur la ligne `message`. Sans UIDPLUS, ou chez un fournisseur qui copie lui-même, la ligne n'a pas d'UID : la synchronisation de Sent la rattache par son `Message-ID` à son prochain passage, comme après un déplacement sans UIDPLUS (`modele de donnees.md`, section 6). La synchronisation ne doit donc pas créer une seconde ligne pour ce mail.
- Un compte sans dossier Sent : le mail est envoyé, aucune ligne n'est créée (un dossier local sans existence sur le serveur serait signalé comme disparu par la synchronisation) ; l'action se termine avec un avertissement, et la ligne `send` de l'historique en garde la trace.
- Un échec de l'`APPEND` (serveur IMAP injoignable) ne remet pas en cause l'envoi : la même action `send`, passée en phase `append`, est reprise avec les délais du spool. Après la dernière tentative, elle se termine sans copie sur le serveur.
- Tant qu'elle n'a pas d'UID, une ligne envoyée par NiceMailer depuis moins d'une heure n'est pas prise pour une orpheline par la synchronisation : la copie faite par le fournisseur peut tarder.

## 10. Écrire à quelqu'un vaut décision Inbox

À l'acceptation SMTP, chaque destinataire (À, Cc, Cci) **sans décision** reçoit la décision `inbox`, avec la source du client et une ligne `screening` dans `audit_log` de raison `sent_to` (`dossiers.md`, « Écrire à quelqu'un vaut décision Inbox »). Les mails de ces destinataires encore en Quarantine sont rangés dans l'Inbox. Les décisions existantes, y compris Banned, sont conservées. Les adresses du compte sont exclues.

Ces décisions font partie de la même commande que le rangement : `z` après l'envoi les annule avec lui.

## 11. Contrat

| Commande | Paramètres | Résultat |
| --- | --- | --- |
| `create_draft` | `reply_to` (id de message, qui désigne aussi le compte), `quote_header` (ligne d'introduction mise en forme par le client), `body?` (texte d'un brouillon abandonné à restaurer) | `{draft, reused}` ; si un brouillon de réponse à ce mail existe, c'est lui (`reused: true`) ; `BODY_PENDING` si le corps du mail n'est pas encore téléchargé (il est demandé en priorité), `CANNOT_SEND` pour un compte qui ne peut pas envoyer |
| `open_draft` | `id` | `Draft` |
| `update_draft` | `id`, `revision`, `body` | `{revision}` ; erreur `REVISION_CONFLICT` si la révision n'est pas la courante, `DRAFT_SENDING` si le brouillon est `sending` |
| `close_draft` | `id` | — |
| `discard_draft` | `id` | `Draft` abandonné, que le client garde pour le restaurer |
| `send_draft` | `id`, `revision`, `body`, `after_send?` (`archive`, `waiting`, `todo`, `stay` ; enregistré dans le brouillon ; absent : le choix enregistré, sinon la présélection) | `{send_at}` : heure prévue de l'envoi ; `CANNOT_SEND` sans destinataire ou sans `[smtp]` |
| `cancel_send` | `id` | `Draft` remis à `editing` ; erreur `SEND_ALREADY_STARTED` |

`Draft` : `id`, `account_id`, `from` (`{addr, name}`), `to`, `cc` (listes de `{addr, name}`), `subject`, `body`, `revision`, `state`, `reply_to`, `after_send_target` (présélection, absente quand rien n'est rangé), `after_send` (choix de l'utilisateur, absent s'il n'y en a pas), `send_at` (si `sending`).

`quote_header` est mis en forme par le client parce que la langue et le format de date sont ceux de l'app ; le noyau ne la modifie pas. Les clients sans langue (CLI) passent une ligne anglaise.

Événements : `DraftSent {draft_id, message_id, thread_id, moved_to?, undo?}`, `SendFailed {draft_id, reply_to, reason}`, `SendPartial {draft_id, refused}`.

Ces commandes sont réservées à l'app, à la TUI et au CLI : un agent (MCP) ou le serveur web reçoit `FORBIDDEN`, en attendant la confirmation des envois d'agents dans l'app (`architecture.md`, section 9).

**Envoi simulé.** Avec `NICEMAILER_FAKE_NETWORK` (scénarios `make ui-test`, essais sur une base réelle), un compte absent d'`accounts.toml` (le faux courrier de `seed`, ou un compte d'archives importé d'un mbox) répond depuis son adresse, et ses mails sont enregistrés comme acceptés à l'échéance, sans aucun réseau. Les comptes déclarés dans `accounts.toml` envoient réellement.

## 12. Réglages

| Clé | Défaut | Rôle |
| --- | --- | --- |
| `compose.undo_send_seconds` | 5 | Délai d'annulation de l'envoi, de 0 à 30 |
| `accounts.<clé>.save_sent` | `auto` | Copie dans Sent : `auto` (section 9), `always`, `never` |

## 13. Tests

- **`nm-core`** : destinataires (Reply-To, réponse à son propre mail, identités retirées, doublons et casse, À vide, personne), adresse d'expédition, objet (préfixes, numéros, langues, vide), `References` (bornage, racine gardée), citation (niveaux, lignes vides, troncature), `format=flowed` (coupure, citations, espaces de fin), détection du fournisseur, présélection.
- **`nm-store`** : cycle de vie du brouillon (création, réutilisation, révision, abandon et restauration, `sending` non modifiable), fin d'envoi en une transaction (ligne Sent, rangement, décisions, suppression du brouillon), rejouée sans doublon.
- **`nm-engine`**, contre le Dovecot de `dev/` avec un service de soumission SMTP : envoi réel, copie dans Sent avec `APPENDUID`, rattachement par la synchronisation sans doublon, échec temporaire puis reprise, `5xx` définitif, destinataire refusé, délai d'annulation et `cancel_send`, plantage pendant le délai.
- **`nm-daemon`** : les commandes par le socket, erreurs et événements.
- **App** : tests unitaires (`GeekController` contre `FakeCore` : `r`, Échap, `⌘↩`, `⌘⌫`, `z` pendant le délai et après) et scénario `.uiscript` (répondre, taper, envoyer, voir la conversation rangée).

## 14. Questions ouvertes

- **Brouillons locaux dans Drafts** : faut-il, avant `append_draft`, créer une ligne `message` locale pour les lister dans Drafts ?
- **Listes de diffusion** : répondre à tous sur une liste (`List-Post`) envoie à la liste et à l'auteur ; faut-il retirer l'auteur quand il est abonné ?
- **Rangement de la réponse depuis Archive** : une relance depuis Archive devrait-elle remettre la conversation en Waiting ?
