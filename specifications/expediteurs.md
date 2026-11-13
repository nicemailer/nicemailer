# NiceMailer : personnes et robots

Statut : validé le 2026-11-13. Complète `dossiers.md` (screening), `ui.md` (sélecteur d'attributs, écran Contacts) et `domaines-publics.md`.

## 1. Objet

Séparer les mails écrits par des personnes (`alice.dupont@gmail.com`) de ceux qu'envoient des automates (`info@amazon.fr`, mais aussi `alice.dupont@la-boutique.com` quand la boutique envoie sa lettre d'information par Brevo sous le nom d'une vendeuse).

Newsletter et Paperwork ne contiennent presque que des robots. Inbox et Archive mélangent les deux : c'est là que la séparation sert, sous la forme d'un **filtre** de la liste, d'usage fréquent. Aucun dossier n'est créé et aucun mail n'est déplacé : une erreur de classement ne coûte qu'un affichage.

## 2. Principe

- Le **verdict porte sur l'adresse** de l'expéditeur : `person` (personne) ou `robot`. Il complète la table `address`, à côté de la décision de screening, dont il est indépendant.
- Il est calculé à partir d'**indices relevés dans les en-têtes** de chacun de ses mails, à la réception : les en-têtes suffisent, le corps n'est pas lu. Le classement est donc connu dès la synchronisation des en-têtes.
- L'utilisateur peut **forcer** le verdict d'une adresse depuis l'écran Contacts. Le verdict forcé l'emporte toujours sur le calcul.
- Le verdict est **recopié dans chaque message** (`message.sender_kind`) et résumé **dans chaque conversation** (`thread.has_person`) : c'est un critère fréquent, le filtre teste une colonne pendant le parcours de la liste, sans jointure ni calcul d'ensemble.
- Un mail qui est lui-même automatique (lettre d'information, réponse automatique) est `robot` même quand son adresse est celle d'une personne : une petite entreprise envoie sa newsletter depuis l'adresse où écrit son gérant (section 6, « Adresses mixtes »).
- Les **règles sont déterministes et explicables**, sans apprentissage automatique : on peut toujours dire pourquoi une adresse est classée robot.

## 3. Mesures

Les règles ont été construites sur une vraie boîte, en lecture seule, le 2026-11-13 : 399 864 messages, 22 900 expéditeurs. Trois groupes de référence existaient déjà :

- robots : les mails de Newsletter et de Paperwork, et les adresses de décision `newsletter` ou `paperwork` (55 adresses, 19 000 mails) ; Banned (113 adresses) ;
- personnes : les adresses à qui l'utilisateur a écrit (2 677 adresses, 151 000 mails).

| Indice (part des mails) | Newsletter | Paperwork | Écrit à |
| --- | --- | --- | --- |
| Logiciel de messagerie (`User-Agent`, `X-Mailer`) | 0 % | 0 % | 42 % |
| `Message-ID` de Gmail | 0 % | 0 % | 22 % |
| Réponse (`In-Reply-To`, `References`) | 0,5 % | 0 % | 48 % |
| Partie locale `noreply` | 3 % | 78 % | 0 % |
| Partie locale de service (`info`, `contact`…) | 80 % | 8 % | 4 % |
| `List-Unsubscribe` | 21 % | 3 % | 7 % |

Les mails transactionnels (Paperwork) n'ont presque jamais `List-Unsubscribe` : c'est l'adresse `noreply` qui les signe.

Résultat des règles de la section 5 :

| Groupe | Personne | Robot |
| --- | --- | --- |
| Newsletter, Paperwork | 4 adresses | 51 adresses (83 % des mails) |
| Écrit à | 2 581 (96 %) | 74 |
| Banned | 5 | 109 |

Sur un tirage au hasard des adresses à qui l'utilisateur n'a jamais écrit, environ 94 % des verdicts `robot` sont justes. Les erreurs restantes sont décrites à la section 7.

## 4. Indices d'un message

Relevés dans les en-têtes, une fois, à la réception (`ingest.rs`). Chaque indice est un bit de `message.sender_signals`.

**Indices de masse** : un seul suffit à faire du mail un mail de masse.

- `List-Unsubscribe`, hors liste de discussion ;
- `Feedback-ID` ;
- `Precedence: bulk` ou `junk`, hors liste de discussion ;
- `Auto-Submitted: auto-generated` accompagné d'un autre indice de masse (seul, voir « Réponse automatique ») ;
- partie locale `noreply` sous toutes ses formes : `no-reply`, `no_reply`, `donotreply`, `ne-pas-repondre`, `nepasrepondre`, `pasdereponse`, `mailer-daemon`, `bounce`…, y compris au milieu (`avisage-ne-pas-repondre@chronopost.fr`) ;
- `X-Mailer` d'une plateforme d'envoi ou d'un logiciel serveur : Mailchimp, Sendinblue, Brevo, ActiveCampaign, Customer.io, Adobe Campaign (`nlserver`), Sendy, Mailjet, SendGrid, HubSpot, Salesforce, Zendesk, WordPress, Symfony, Nodemailer, JavaMail…

**Liste de discussion** : `List-Post` (sauf `NO`), `X-BeenThere`, ou un `List-Id` en `googlegroups.com`.

**Indices de personne** :

- `User-Agent` ou `X-Mailer` d'un logiciel de messagerie : Apple Mail, Outlook, Thunderbird, Roundcube, Mutt…, ou d'un navigateur (`Mozilla/`) ;
- `Message-ID` de Gmail (`…@mail.gmail.com`) ;
- réponse : `In-Reply-To` ou `References`.

**Relais de plateforme** : un en-tête propre aux plateformes d'envoi (`X-SES-`, `X-SG-`, `X-Mailgun-`, `X-MC-User`, `X-Mandrill-`, `X-PM-`, `X-CSA-Complaints`, `X-Campaign`…). Il ne compte que s'il n'y a aucun indice de personne : des personnes envoient par Mandrill.

**Réponse automatique** : `Auto-Submitted: auto-replied`, `X-Autoreply`, `X-Autorespond`, ou `Auto-Submitted: auto-generated` seul (réponse d'Exchange à une invitation de calendrier).

La **nature du message** en découle, dans cet ordre : réponse automatique, masse, liste de discussion, personne, relais de plateforme (compté comme masse), aucun indice.

### Pièges relevés par les mesures

- **Listes de discussion.** Un Google Group de 399 participants porte `List-Unsubscribe` sur ses 7 054 mails. La liste de discussion est donc reconnue avant les indices de masse.
- **Passerelles de sécurité.** Trend Micro (`X-TM-…`), Mimecast (`X-MC-Unique`) et Proofpoint (`X-EMS-…`) ajoutent des en-têtes qui ressemblent à ceux des plateformes. La liste des en-têtes de plateforme est faite de préfixes précis : `X-MC-User`, pas `X-MC-`. Sans cette précaution, un correspondant à qui l'utilisateur a écrit 382 fois était classé robot.
- **PHPMailer** sert aussi de webmail à de vraies personnes (816 mails d'une seule) : ce n'est pas un indice.
- **`Auto-Submitted: auto-generated`** est posé par Exchange sur les réponses aux invitations : seul, il ne fait pas un robot.

## 5. Verdict d'une adresse

Les mails de l'adresse sont comptés par nature (`address.sender_*`), les réponses automatiques à part. Les règles s'appliquent dans l'ordre ; la première qui conclut décide.

1. Verdict forcé par l'utilisateur : il décide.
2. Une adresse de l'utilisateur (`account.address`) : **personne**.
3. Aucun mail compté (que des réponses automatiques, ou aucun mail reçu) : **personne** si l'utilisateur lui a écrit, sinon **robot**.
4. Au moins la moitié des mails viennent d'une liste de discussion : **personne**. Ce sont des gens qui écrivent ; l'utilisateur peut forcer l'adresse s'il en juge autrement.
5. L'utilisateur lui a écrit : **personne** si au moins 10 % de ses mails sont de personne ou si moins de la moitié sont de masse ; sinon **robot**. Une newsletter à laquelle on a répondu une fois reste un robot ; un commerçant qui écrit lui-même et envoie aussi sa lettre d'information reste une personne.
6. Au moins la moitié des mails sont de masse : **robot**.
7. Une partie locale de service (`info`, `contact`, `support`, `hello`, `newsletter`, `notifications`, `factures`…, liste dans `sender_kind.rs`) : **robot**. Les services clients sur ticket (Zendesk, `desk@`, `support@`) sont donc des robots, sauf ceux à qui l'utilisateur a écrit (règle 5).
8. Au moins la moitié des mails viennent d'un logiciel de messagerie ou de Gmail : **personne**.
9. Aucun mail de masse, et un domaine public (`domaines-publics.md`) ou au moins la moitié de réponses : **personne**.
10. Sinon **robot**. Un expéditeur à qui l'utilisateur n'a jamais écrit, sans aucune trace de personne, est presque toujours un automate : répondeur vocal, script de déploiement, notifications anciennes sans en-tête de liste.

« L'utilisateur lui a écrit » : l'adresse est destinataire (To, Cc, Bcc) d'un mail d'un dossier Sent (`address.written_to`).

## 6. Verdict d'un message

`message.sender_kind` vaut :

- `person` pour un mail de l'utilisateur (dossiers Sent et Drafts) : c'est une personne qui l'a écrit, et une conversation où l'utilisateur a répondu est une conversation entre personnes ;
- `robot` pour un mail automatique lui-même (masse, relais de plateforme, réponse automatique), quelle que soit son adresse ;
- sinon le verdict de son adresse.

Un mail sans expéditeur lisible n'a pas de verdict (`NULL`) : le filtre le range avec les robots.

**Adresses mixtes.** 163 adresses classées personne ont au moins un mail de masse (1 643 mails), par exemple une galerie qui écrit à l'utilisateur et lui envoie aussi sa lettre d'information depuis la même adresse. Le filtre « humain » montre ses mails personnels et pas sa newsletter.

Quand le verdict d'une adresse change (calcul ou choix de l'utilisateur), tous ses mails sont mis à jour dans la même transaction, par l'index `idx_message_from`.

## 7. Limites acceptées

- **Le spam rédigé par des humains** (offres de référencement depuis Hotmail ou Gmail) est classé personne. Le screening protège les nouveaux mails (Quarantine) ; dans les anciennes archives, il est accepté.
- **Une lettre d'information envoyée depuis l'Outlook d'une personne**, sans aucun en-tête de liste, est classée personne : rien ne la distingue techniquement d'un mail ordinaire.
- **Le verdict forcé n'est pas encore synchronisé entre Mac.** Il le sera avec les décisions de screening (`synchronisation-configuration.md`, section 3).

## 8. Filtre de la liste

Le sélecteur d'attributs (`ui.md`) propose un attribut oui / non : **humain** / **robot** (anglais : *human* / *robot*).

- **humain** : un mail de la conversation, dans n'importe quel dossier, est `person` ;
- **robot** : aucun ne l'est. Les deux valeurs se partagent la liste, sans recouvrement ni oubli.

N'importe quel dossier, et pas seulement les dossiers listés : une notification à laquelle l'utilisateur a répondu est une conversation entre personnes, et sa réponse est dans Sent, pas dans Archive.

Le critère est la colonne `thread.has_person`, testée pendant le parcours de l'index de la liste. Mesuré sur 400 000 mails : 2 ms pour trois pages d'Archive, comme sans filtre. Un ensemble de conversations calculé à chaque requête, comme pour les pièces jointes, coûtait 30 à 160 ms.

## 9. Écran Contacts

Chaque adresse affiche son verdict (« humain » ou « robot »), marqué quand il est forcé. La touche `o` (*origine*) ouvre le choix : `h` humain, `r` robot, `-` automatique (le calcul reprend la main). Le choix est journalisé (`audit_log`, action `sender_kind`) et annulable.

## 10. Données

| Colonne | Rôle |
| --- | --- |
| `message.sender_signals` | indices du message (bits, section 4) ; `NULL` tant que le rattrapage ne l'a pas lu |
| `message.sender_kind` | `person`, `robot`, `NULL` sans expéditeur |
| `address.sender_kind` | verdict calculé |
| `address.sender_kind_user` | verdict forcé, `NULL` sinon |
| `address.written_to` | l'utilisateur a écrit à l'adresse |
| `address.sender_count`, `sender_bulk`, `sender_list`, `sender_human`, `sender_client` | mails comptés par nature (section 5) |
| `thread.has_person` | un mail de la conversation est `person` (section 8) |

Les compteurs sont mis à jour à chaque mail reçu : recalculer un verdict ne relit jamais les mails de l'adresse.

**Mails déjà en base.** La migration vers la version 12 ajoute les colonnes et remplit `address.written_to` (environ 3 s sur 400 000 mails). Un rattrapage en tâche de fond relit ensuite les en-têtes des messages bruts (le début du fichier seulement) et traite chaque mail comme à la réception : indices, compte de l'adresse, verdict, conversation. Il avance par lots de 500 mails, chacun dans sa transaction : un arrêt en cours ne perd qu'un lot, repris au démarrage suivant. Mesuré sur 400 000 mails : 22 s, au plus 120 ms par lot. Tant qu'il n'a pas fini, les mails non encore lus n'ont pas de verdict et le filtre les range avec les robots.
