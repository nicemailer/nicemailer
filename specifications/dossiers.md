---
date: 2026-10-17
---

## Introduction : un parti pris
niceMailer propose un parti pris sur la façon de gérer les emails et ne cherche pas à être un client imap complet.

C'est dans la gestion des dossiers que c'est le plus visible : seuls quelques dossiers prédéfinis sont gérés par le workflow de l’application et les autres dossiers éventuellement présents sur le serveur IMAP ne sont pas gérés dans ce workflow.

La notion de dossier - le système de classement historique d'IMAP -  est une représentation directe du système de fichiers sous-jacent des serveurs. Ce système a montré ses limites, car un même mail ne peut pas être dans plusieurs dossiers.
Et c'est notamment l'une des raisons du succès initial de Gmail qui a privilégié les notions d'étiquettes plutôt que de dossier : un même mail peut avoir plusieurs étiquettes alors qu'il aurait été limité à un seul dossier. 

niceMailer fait un usage très cadré des dossiers : ils représentent les grandes catégories d’emails, mais surtout il concrétise le workflow de l'application.
Pour une classification plus fine des mails, l'application propose un système de tags (étiquettes, labels, mots-clés).

Les dossiers de niceMailer restent cependant de "vrais" dossiers IMAP, afin qu'en mobilité, l'utilisateur puisse accéder à son compte IMAP (par exemple sur l'application de son téléphone) en bénéficiant du workflow de niceMailer. 

Souvent, lorsqu'on est en mobilité, son ordinateur principal reste allumé à la maison ou au bureau, et niceMailer pourra donc continuer à nettoyer l’inbox, offrant ainsi une expérience de consultation confortable à tout moment. C'est le comportement par défaut : l'option « Continuer à trier mes mails quand NiceMailer est fermé » est activée, et le noyau tourne dès l'ouverture de session, même si l'app n'a pas été ouverte (voir `architecture.md`, section 6).

## Les dossiers de niceMailer

I Inbox
	Mails entrants que le screener n'a pas classés dans un autre dossier

T Todo
	Mails a traiter. Les mails arrivent dans cette boite par une commande de l’utilisateur
W Waiting
	Mails en attente d'une réponse/information complémentaire. Les mails arrivent dans cette boite par une commande de l’utilisateur

A Archive
	Mails traités : par une commande de l'utilisateur, ou par le rangement choisi à l'envoi d'une réponse (voir « Répondre à une conversation »)
S Sent
	Mails envoyés par l'utilisateur
R Drafts
	Projets de mails en cours de rédaction. Ce dossier existe côté serveur IMAP pour que l'utilisateur retrouve ses brouillons sur son téléphone, et inversement. Un brouillon est un message portant le flag `\Draft` (attribut `is_draft` en base). Il quitte ce dossier à l'envoi, le mail étant alors rangé dans Sent.

N Newsletter
	Newsletters
P Paperwork
	Factures, alertes, etc

Q Quarantine
	Mails d'expéditeur inconnu à classer
D Deleted
	Mails supprimés par l'utilisateur

X Junk
	Spam
B Banned
	Mails d'expéditeur bloqué par l'utilisateur



La lettre de chaque dossier sert de raccourci clavier pour y déplacer un mail. Les lettres H, J, K et L sont réservées à la navigation à la manière de Vim, d'où X pour Junk.

## Le fonctionnement des dossiers

NiceMailer classifie les mails entrants en fonction de l'expéditeur.

Les mails d'expéditeurs inconnus vont dans le dossier Quarantine où l'utilisateur pourra décider de bannir cet expéditeur (Banned), de mettre le mail dans Junk s'il s'agit de spam, ou de le déplacer vers une cible légitime : Inbox, Newsletter, Paperwork (voir « Déplacer un mail »). Le fait de déplacer un mail de Quarantine vers une cible légitime est mémorisé par l’application et les prochains mails de cet expéditeur seront automatiquement livrés dans le dossier approprié. Les autres mails déjà reçus de cet expéditeur sont rangés en même temps (voir « Décision sur un expéditeur »).

Cela reprend le workflow popularisé par Hey de 37signals.

Les autres dossiers correspondent à des commandes de l'utilisateur sur un mail : Todo, Waiting, Archive, Deleted, Junk. Drafts est alimenté par la rédaction d'un message, et Sent par son envoi.

Pendant la rédaction, le brouillon est enregistré en continu en local (table `draft`). Il n'est envoyé au serveur IMAP, dans Drafts, qu'à la fermeture de la fenêtre de rédaction. Un brouillon modifié à la fois sur le Mac et sur un autre appareil est conservé en deux exemplaires, pour ne jamais perdre de texte. Les pièces jointes sont copiées dans la zone de stockage privée dès qu'on les attache. Le détail est dans `brouillons.md`.

Un mail sorti de Quarantine ne peut pas y retourner.

## Déplacer un mail

### Une conversation, pas un mail

Les commandes s'appliquent à la **conversation** : tous ses mails sont traités de la même façon. Une conversation n'occupe donc qu'un seul vrai dossier, en plus des dossiers techniques (« Dossier d'une conversation »).

Une commande sur une conversation (déplacer, supprimer, marquer comme spam) s'applique à ses mails rangés dans Inbox, Todo, Waiting, Archive, Newsletter et Paperwork. Elle ne touche jamais :

- **Sent et Drafts** : les mails envoyés restent dans Sent, où le téléphone les retrouve, sauf ceux d'une conversation qui n'a rien d'autre à déplacer (voir « Cibles proposées ») ; un brouillon en cours n'est pas déplacé. Inversement, supprimer un brouillon depuis Drafts (voir « Cibles proposées ») ne touche que les brouillons, pas la conversation à laquelle ils répondent ;
- **Deleted, Junk et Banned** : un mail que l'utilisateur a supprimé ou rejeté n'est pas ressuscité par une action sur le reste de la conversation ;
- **Quarantine** : un mail en Quarantine attend une décision sur son expéditeur, que la commande ne prend pas à sa place.

Autres règles :

- **Lu, non lu** : s'applique à tous les mails de la conversation. **Drapeau** : posé sur le dernier mail reçu ; la conversation est marquée si l'un de ses mails l'est.
- **Décision sur l'expéditeur** (déplacement vers Newsletter, Paperwork ou Banned, ou vers Inbox depuis Quarantine ou Banned) : la décision porte sur l'auteur du dernier mail reçu de la conversation, hors ceux de l'utilisateur (Sent, Drafts, ou envoyés depuis l'adresse du compte). Les autres expéditeurs de la conversation gardent leur décision. Une conversation qui ne contient que des mails de l'utilisateur ne crée aucune décision. Choix assumé : demander quels expéditeurs sont concernés a été écarté pour garder une commande d'une touche ; le risque est de bannir un collègue en copie s'il est l'auteur du dernier mail, et l'annulation (« Annulation ») le rattrape.
- **Nouveau mail dans une conversation déjà rangée** : il arrive là où le screening le place, en général l'Inbox. Toute la conversation revient alors dans l'Inbox (son dossier est celui de son dernier mail, « Dossier d'une conversation ») et quitte la liste de son ancien dossier ; ses anciens mails restent dans leur dossier sur le serveur, où le téléphone les voit, jusqu'à la commande suivante, qui regroupe tous ses mails. Exception : une conversation en Todo ou en Waiting qui reçoit un mail revient entièrement dans l'Inbox, sur le serveur aussi (« Nouveau mail dans une conversation en Todo ou en Waiting »).
- **Mail isolé** : dans la vue de lecture, un mail seul d'une conversation peut être supprimé. C'est la seule commande au niveau du mail.

Les commandes du contrat prennent donc un identifiant de conversation (`thread_id`) ; seule la suppression accepte aussi un identifiant de message.

### Dossier d'une conversation

Une conversation n'est pas éparpillée entre plusieurs vrais dossiers : elle est dans **un** dossier, celui qui dit ce qu'elle est (à traiter, classée, rejetée). Les dossiers techniques ne contiennent que certains de ses mails, pour des raisons techniques : le mail envoyé dans Sent, le brouillon dans Drafts, le mail d'un expéditeur inconnu dans Quarantine, le mail supprimé seul dans Deleted. Ajouté en novembre 2026, avec la version 9 du schéma (`modele de donnees.md`, « `thread` »).

Le dossier d'une conversation est, du premier cas qui s'applique au dernier :

1. le dossier de son dernier mail rangé dans Inbox, Todo, Waiting, Archive, Newsletter ou Paperwork ;
2. sinon, celui de son dernier mail rangé dans Junk ou Banned ;
3. sinon, dans cet ordre fixe, Sent, Drafts, Quarantine ou Deleted, le premier qui contient un de ses mails ;
4. sinon aucun : ses mails sont tous dans des dossiers sans rôle (un dossier qui a perdu le sien). Elle n'apparaît que dans Everything.

- **Pourquoi Junk et Banned après les autres vrais dossiers.** Une commande sur la conversation ne touche jamais ses mails de Junk et de Banned (« Une conversation, pas un mail ») : un mail rejeté qui y reste ne dit plus ce qu'est la conversation. Sans cette règle, le mail d'un expéditeur banni dans une conversation de l'Inbox ferait passer toute la conversation dans Banned, et elle disparaîtrait de l'Inbox.
- **Pourquoi Quarantine avant Deleted.** Deleted est le seul repli qui veut dire « plus vivant » : une conversation en Quarantine dont un mail a été supprimé reste dans All Mail, où son mail attend une décision.
- **Dossiers techniques : des drapeaux.** Une conversation a un drapeau par dossier technique (`has_sent`, `has_draft`, `has_quarantine`, `has_deleted`) : elle a au moins un mail dans ce dossier. Les listes de Sent, Drafts, Quarantine et Deleted lisent ces drapeaux. Une conversation de l'Inbox à laquelle l'utilisateur a répondu est donc listée dans l'Inbox et dans Sent ; une conversation de l'Inbox qui reçoit un mail d'un inconnu (resté en Quarantine) est listée dans l'Inbox et dans Quarantine.
- **Ce qu'affiche une ligne de liste.** Partout, le nombre de mails de toute la conversation et la date de son dernier mail, envoyé ou reçu, sans compter ceux de Deleted, Junk et Banned ; une conversation qui n'a que de tels mails les compte tous. Les compteurs propres à un dossier ont été abandonnés : la liste Sent est triée par le dernier mail de la conversation, pas par le dernier envoi (choix assumé, novembre 2026).
- **Une conversation par compte.** Le même mail reçu sur deux comptes (deux adresses de l'utilisateur, un renvoi d'un compte vers un autre) fait deux conversations, une dans chaque compte : chaque compte a ses dossiers sur son serveur, et une commande ne touche que les mails de son compte.

### Catégories de dossiers

| Catégorie | Dossiers | Rôle |
| --- | --- | --- |
| Travail | Inbox, Todo, Waiting | Mails à traiter |
| Rangement | Archive, Newsletter, Paperwork | Mails classés |
| Rejet | Junk, Banned | Spam et expéditeurs bloqués |
| Techniques | Sent, Drafts, Quarantine, Deleted | Statuts techniques : on n'y déplace pas un mail, sauf Deleted (voir ci-dessous) |

### Cibles proposées

La commande de déplacement propose, selon le dossier courant :

| Dossier courant | Cibles proposées |
| --- | --- |
| Quarantine | Inbox, Newsletter, Paperwork, Junk, Banned, Deleted |
| Travail et rangement | Todo, Waiting, Inbox, Archive, Newsletter, Paperwork, Junk, Banned, Deleted, sauf le dossier courant |
| Junk, Banned | Mêmes cibles que les dossiers de travail et de rangement, pour rattraper une erreur |
| Deleted | Mêmes cibles, pour restaurer un mail supprimé |
| Sent | Mêmes cibles que les dossiers de travail et de rangement (ajouté en novembre 2026) : une conversation listée dans Sent se range depuis Sent comme depuis son dossier. Ses mails de Sent y restent, seul le reste de la conversation est déplacé ; elle reste donc listée dans Sent. Une conversation qui n'a que des mails dans Sent (un nouveau mail, encore sans réponse) n'aurait rien à déplacer : ses mails de Sent partent alors dans la cible, ici et sur le serveur, et elle quitte Sent. Choix assumé : le téléphone ne voit plus ce mail dans Sent, et s'il est en Waiting, la réponse le ramène avec la conversation dans l'Inbox (« Nouveau mail dans une conversation en Todo ou en Waiting »), pas dans Sent. Une copie (le mail dans Sent et dans la cible) a été écartée : deux mails de même `Message-ID` dans la conversation, à dédoublonner partout |
| Drafts | Deleted seulement (ajouté en novembre 2026) : Drafts est alimenté par la rédaction, mais un brouillon doit pouvoir être supprimé, y compris celui d'un autre logiciel (Spark, le téléphone) que NiceMailer n'ouvre pas encore. Seuls les mails de Drafts de la conversation partent dans Deleted, ici et sur le serveur. Un brouillon NiceMailer y perd sa ligne `draft` (son texte reste dans le mail de Deleted) ; c'est refusé tant qu'il est en cours d'envoi ou que sa copie vers le serveur n'est pas faite. Toute autre cible est refusée avec un message qui dit d'utiliser Suppr |

**Deleted** est un dossier technique, mais il est proposé comme cible pour fluidifier la commande : déplacer un mail dans Deleted et le supprimer (touche Suppr) sont la même action.

### Déplacement et décision sur l'expéditeur

Certains déplacements mémorisent une décision sur l'adresse de l'expéditeur ; les autres ne concernent que le mail déplacé.

| Déplacement | Effet |
| --- | --- |
| Vers Newsletter, Paperwork ou Banned, depuis n'importe quel dossier | Décision sur l'adresse, avec rangement des autres mails (« Décision sur un expéditeur ») |
| Vers Inbox, depuis Quarantine ou Banned | Décision `inbox` sur l'adresse, avec rangement |
| Vers Inbox depuis un autre dossier, vers Todo, Waiting, Archive ou Deleted | Ce mail seulement |
| Vers Junk | Ce mail seulement, même depuis Quarantine |

- **Pourquoi une décision depuis tous les dossiers.** Si ranger une newsletter de l'Inbox dans Newsletter ne créait pas de décision, la suivante reviendrait dans l'Inbox.
- **Junk ne crée jamais de décision.** Le spam change d'adresse d'expédition à chaque envoi : une règle sur l'adresse ne servirait à rien. L'expéditeur reste sans décision, et un nouveau mail de sa part repasse par la Quarantine. Banned, à l'inverse, vise un expéditeur réel et identifié que l'utilisateur ne veut plus lire.
- **Mail suspect** (`auth_suspect = 1`, usurpation probable) : aucun déplacement ne crée de décision sans confirmation explicite (`authentification-expediteur.md`, section 5). Le mettre dans Junk ou Banned ne concerne que ce mail : bannir l'adresse réelle à cause d'un usurpateur serait une erreur.

### Depuis la Quarantine

Le plus souvent, la Quarantine contient les premiers mails d'un expéditeur inconnu : la décision s'applique alors à ce mail et aux autres mails du même expéditeur encore en Quarantine. Mais des mails de cet expéditeur peuvent aussi exister ailleurs :

- un expéditeur de l'historique importé, sans décision, dont les anciens mails sont dans l'Inbox ou l'Archive (`import-initial.md`) ;
- un expéditeur connu dont un mail a été mis en Quarantine parce que son authentification a échoué ;
- un expéditeur dont la décision a été retirée.

La règle de rangement s'applique donc aussi depuis la Quarantine : c'est elle qui range l'historique importé au fil des décisions.

### Répondre à une conversation

Répondre fait souvent avancer le traitement d'une conversation, mais le noyau ne peut pas deviner vers quoi : une réponse peut clore l'échange (« merci, c'est noté »), relancer l'attente (une nouvelle question) ou laisser un travail à faire. L'écran de réponse demande donc **où ranger la conversation après l'envoi** (`⌘↩` applique la présélection, `⇧⌘↩` la fait choisir, `reponse.md` section 8), parmi trois cibles : **Archive**, **Waiting** ou **Todo**. La cible la plus probable est présélectionnée, et l'utilisateur n'a rien à faire dans le cas courant.

| Dossier de la conversation | Cible présélectionnée | Raison |
| --- | --- | --- |
| Inbox | Archive | Répondre est le plus souvent la dernière étape du traitement |
| Inbox, revenue de Waiting (`$NmWasWaiting`) | Waiting | La réponse attendue est arrivée, mais l'échange continue souvent ; une conversation archivée à tort sort du suivi sans que l'utilisateur s'en aperçoive |
| Inbox, revenue de Todo (`$NmWasTodo`) | Todo | Le travail existait avant le nouveau mail ; une réponse le termine rarement |
| Todo | Archive | Répondre depuis Todo, c'est en général faire ce qui était à faire |
| Waiting | Waiting | Répondre depuis Waiting est une relance : on attend toujours |

Une quatrième option, à l'essai, « laisser en place », envoie sans toucher à la conversation (`reponse.md`, section 8).

Le choix n'est pas proposé pour les autres dossiers, où la conversation reste en place :

- **Newsletter, Paperwork** : le classement reste valable ;
- **Archive** : la conversation est déjà traitée ;
- **Quarantine** : écrire à l'expéditeur lui donne la décision `inbox` (« Écrire à quelqu'un vaut décision Inbox »), qui remet ses mails dans l'Inbox.

Règles :

- **Réponse** : réponse ou réponse à tous, c'est-à-dire un mail dont `In-Reply-To` désigne un mail de la conversation. La conversation concernée est celle du mail auquel on répond. Un transfert n'est pas une réponse et ne range rien.
- **Conversation, pas mail** : comme une commande, le rangement s'applique aux mails de la conversation rangés dans Inbox, Todo et Waiting (« Une conversation, pas un mail »). Si la cible est le dossier actuel (relance depuis Waiting), rien n'est déplacé.
- **Moment** : quand le serveur SMTP a accepté le mail, pas à l'ouverture de la fenêtre de rédaction. Un envoi qui échoue ne range rien.
- **Marque de retour** : le rangement est une commande, il retire donc `$NmWasTodo` et `$NmWasWaiting` (« Nouveau mail dans une conversation en Todo ou en Waiting »), y compris quand la conversation ne change pas de dossier.
- **Présélection calculée par le noyau** : la règle du tableau vit dans le noyau, pas dans chaque client. Le contrat expose la cible présélectionnée avec la conversation (`ThreadSummary.reply_target`, absent pour les dossiers sans choix), et la commande d'envoi (`send_draft` ou `send_message`, voir `brouillons.md`) reçoit la cible choisie (`after_send` : `archive`, `waiting` ou `todo`). Si `after_send` est omis, le noyau applique la présélection : c'est le cas du CLI et de l'agent MCP quand ils ne précisent rien.
- **Réponse envoyée ailleurs** : une réponse envoyée depuis le téléphone ou le webmail, découverte dans Sent par la synchronisation, est rangée selon la présélection, avec la source `core`. Elle n'est vue qu'au prochain passage de la synchronisation sur Sent.
- **Traçabilité** : déplacement ordinaire par mail, dans le spool, avec la source du client qui a envoyé la réponse et une ligne `move` dans `audit_log` qui cite le mail envoyé.

## Implémentation technique

### A la création du compte IMAP dans niceMailer

Lorsqu'un compte est paramétré dans l'application, après avoir fait une connexion pour tester la validité des paramètres fournis. Nicemailer regarde si les dossiers dont il a besoin sont présents sur le serveur. 

S'ils ne sont pas présents, un panneau s'affiche pour éventuellement sélectionner un dossier déjà présent sur le serveur pour chacun des dossiers du workflow, Drafts compris. Si l'utilisateur n'indique pas un dossier préexistant, les dossiers nécessaires sont créés.

Dans l'app, ce panneau est toujours affiché avant la création du compte (section « Sur le serveur » des Préférences, `ui.md`) : il nomme les dossiers qui seront créés, ce qui n'arrive presque jamais à l'insu de l'utilisateur.

Le panneau propose par défaut les dossiers que le serveur déclare avec un attribut d'usage spécial (RFC 6154 : `\Drafts`, `\Sent`, `\Trash`, `\Junk`, `\Archive`), pour Drafts, Sent, Deleted, Junk et Archive. Un dossier Brouillons déjà utilisé par le téléphone est ainsi repris tel quel, et NiceMailer n'en crée pas un second. À défaut d'attribut, la proposition repose sur le nom du dossier, que l'utilisateur confirme.

Règles de la proposition (`nm-engine`, `folders.rs`, déjà visibles avec `nicemailer imap probe`) :

- INBOX est l'Inbox.
- L'attribut d'usage spécial l'emporte sur le nom. `\All` et `\Flagged` désignent des vues d'autres dossiers : jamais un rôle.
- À défaut d'attribut, le nom, en anglais ou en français : `Sent`, `Sent Items`, `Sent Messages`, `Sent Mail`, `Envoyés`, `Messages envoyés`, `Éléments envoyés` ; `Drafts`, `Brouillons` ; `Trash`, `Deleted`, `Deleted Items`, `Deleted Messages`, `Corbeille`, `Éléments supprimés` ; `Junk`, `Spam`, `Junk E-mail`, `Junk Email`, `Courrier indésirable`, `Indésirables`, `Pourriels` ; `Archive`, `Archives`. S'y ajoutent les noms des dossiers propres à NiceMailer : `Todo`, `Waiting`, `Newsletter` (ou `Newsletters`), `Paperwork`, `Quarantine`, `Banned`. La casse ne compte pas.
- Seul un dossier au premier niveau de l'espace de noms personnel compte (`INBOX/Sent` aussi, disposition courante) : `Clients/Sent` est un dossier de l'utilisateur, pas le dossier Sent.
- Un rôle va à un seul dossier : l'attribut, puis le premier nom de la liste, puis le premier dossier listé par le serveur.

Provisoirement, jusqu'au panneau de la phase 7 (`plan-imap.md`), la première synchronisation d'un compte qui n'a encore aucun rôle donne à chaque dossier le rôle proposé par ces règles, sans confirmation ; les dossiers créés ensuite sur le serveur n'en reçoivent pas.

Seule l'Inbox est surveillée en IDLE : c'est par elle que les nouveaux mails arrivent sur le serveur, et c'est là que le screening doit réagir sans délai. IDLE ne surveille qu'un dossier par connexion, et multiplier les connexions est coûteux et limité par les serveurs. Les autres dossiers du workflow sont interrogés de temps à autre, toutes les cinq minutes (paramètre `sync.poll_interval_secs`, 300 par défaut), pour récupérer les changements faits ailleurs, par exemple depuis le téléphone. Les actions faites dans NiceMailer sont appliquées localement tout de suite, sans attendre l'interrogation.

### Regarder avant d'organiser

L'utilisateur peut aussi ajouter le compte **sans rien changer sur le serveur** : aucun dossier créé, aucun mail déplacé, marqué lu ou envoyé, le screening « à blanc » (`read_only`, `configuration.md`, section 6). Il voit ses mails dans NiceMailer avant de décider ; c'est une migration en douceur.

Quand il le décide, depuis la page du compte dans les Préférences, le compte est organisé comme s'il était créé à ce moment-là : même panneau des dossiers, même écran d'import (`import-initial.md`), dossiers créés, frontière du screening reprise au `UIDNEXT` du moment (les mails arrivés pendant la période d'observation sont des mails déjà présents : ils ne sont pas triés), correspondants acceptés, anciens mails archivés, puis `read_only` retiré. Les mails déjà téléchargés ne le sont pas à nouveau : l'archivage les déplace sur le serveur, et la synchronisation reconnaît ces déplacements. Le coût est un passage de plus sur les mails archivés, accepté pour la douceur de la migration.

### Import initial

À la création du compte, les mails déjà présents sur le serveur ne passent pas par le screening : aucun ne part en Quarantine. L'utilisateur peut archiver les plus anciens, et les personnes à qui il a déjà écrit arrivent directement dans l'Inbox. Les autres expéditeurs passent par la Quarantine à leur prochain mail. Le détail est dans `import-initial.md`.

### Données de screening

L'application intègre une table d'adresses (`address`) afin de mémoriser les décisions de screening.
Elle intègre également une table de domaines (`domain`) : l'utilisateur pourra décider pour tous les mails d'un domaine de les classer dans un dossier spécifique. La règle d'adresse l'emporte sur la règle de domaine, et un expéditeur sans règle va en Quarantine, sauf s'il répond dans une conversation connue (ci-dessous).

### Écrire à quelqu'un vaut décision Inbox

Écrire à une personne, c'est l'accepter comme correspondant. Quand un mail est envoyé, chaque destinataire (To, Cc, Bcc) **sans décision** reçoit la décision `inbox`.

- **Une décision existante est conservée.** Écrire à l'adresse de désabonnement d'une newsletter, ou répondre une fois à un expéditeur banni, ne change pas sa décision.
- **Les adresses du compte lui-même** sont exclues : son adresse principale et ses identités (`configuration.md`, section 6).
- **Envois faits ailleurs.** Un mail envoyé depuis le téléphone ou le webmail, découvert dans Sent par la synchronisation, produit le même effet.
- **Rangement.** La décision range les mails de ce destinataire encore en Quarantine vers l'Inbox, comme toute décision `inbox` (« Décision sur un expéditeur »). Si plusieurs Mac font ce rangement en même temps, le déplacement concurrent est traité comme un succès.
- **Traçabilité.** La décision porte la source du client qui a envoyé le mail, ou `imap` pour un envoi découvert dans Sent, et une ligne `screening` dans `audit_log` avec la raison `sent_to`.

C'est la même règle que celle des correspondants à l'import initial, appliquée au fil de l'eau (`import-initial.md`, section 4).

### Réponse dans une conversation connue

Un collègue inconnu mis en copie d'un échange en cours ne doit pas partir en Quarantine : la conversation serait cassée. Un mail d'un expéditeur sans décision est donc livré dans l'Inbox s'il répond à une conversation connue.

**Conversation connue.** Un des identifiants de `In-Reply-To` ou de `References` correspond au `Message-ID` d'un message du même compte rangé dans Inbox, Todo, Waiting, Archive ou Sent. Une conversation qui n'existe qu'en Newsletter, Paperwork, Quarantine, Junk, Banned ou Deleted ne compte pas : une « réponse » d'un inconnu à une newsletter est suspecte. Le contrôle se fait à la synchronisation des en-têtes, par l'index sur `message_id_hdr`, sans attendre le message complet.

**Le mail est accepté, pas l'expéditeur.** Aucune décision n'est enregistrée sur l'adresse. Son prochain mail hors de cette conversation passera par la Quarantine, sauf si l'utilisateur lui répond entre-temps : écrire à quelqu'un vaut décision Inbox (ci-dessus). Dans l'Inbox, le mail porte l'indication « nouveau participant », avec deux actions : accepter l'expéditeur (décision `inbox`) ou l'envoyer en Quarantine. L'indication se déduit des données : un mail de l'Inbox dont l'expéditeur n'a pas de décision.

**Pourquoi pas une décision automatique.** Le détournement de conversation est une attaque connue : un attaquant qui a volé un échange (boîte d'un correspondant compromise) y répond depuis sa propre adresse, avec les bons `References`. Accepter le mail sans accepter l'adresse limite le risque à ce seul mail, signalé comme venant d'un nouveau participant.

**Limites de la règle :**
- une décision Banned (adresse ou domaine) l'emporte toujours ;
- une décision d'adresse ou de domaine existante s'applique normalement : la règle ne concerne que les expéditeurs sans décision ;
- un mail dont le verdict d'authentification est `fail` va en Quarantine avec l'avertissement d'usurpation ;
- les `Message-ID` cités sont faciles à connaître (ils circulent dans les mails transférés et les listes). La règle est un confort, pas une preuve : c'est pourquoi elle n'accepte que le mail.

L'ordre complet des règles est dans `authentification-expediteur.md`, section 4.

### Nouveau mail dans une conversation en Todo ou en Waiting

Ce qui est nouveau et à lire est dans l'Inbox. Quand une conversation rangée dans Todo ou Waiting reçoit un mail, ses mails rangés dans ces dossiers passent dans l'Inbox, avec le nouveau mail. La conversation ne reste pas à moitié dans Todo ou Waiting. Pour Waiting, c'est la fin de l'attente ; pour Todo, l'information nouvelle peut changer ce qu'il y a à faire. Après lecture, l'utilisateur la remet dans Todo ou Waiting s'il y a lieu.

**Déclencheur.** Un mail reçu, livré dans l'Inbox par le screening, qui appartient à une conversation dont des mails sont dans Todo ou Waiting. Cela inclut la réponse d'un nouveau participant acceptée par la règle de la conversation connue (ci-dessus).

**Pas de retour dans l'Inbox :**
- **Mail placé ailleurs par le screening** (Quarantine, notamment après un échec d'authentification, Newsletter, Paperwork, Junk, Banned) : ce mail n'est pas la réponse attendue, ou il est suspect. La conversation reste où elle est.
- **Mail de l'utilisateur lui-même** : il va dans Sent et ne fait pas revenir la conversation dans l'Inbox. Ce que devient la conversation est choisi dans l'écran de réponse (« Répondre à une conversation »).
- **Mail déplacé dans l'Inbox depuis le téléphone ou un autre client** : ce n'est pas une réception, la conversation n'est pas touchée.
- **Import initial** : les mails déjà présents ne passent pas par le screening (`import-initial.md`) et ne déclenchent rien.

**Exécution.** Un déplacement ordinaire par mail, dans le spool, de source `core`, avec une ligne `move` dans `audit_log` qui cite le mail reçu. Lu, non lu et drapeaux sont conservés. Si plusieurs Mac font ce déplacement en même temps, le déplacement concurrent est traité comme un succès. Mac éteint, le nouveau mail attend dans l'Inbox du serveur et la conversation revient au prochain screening.

**Marque de retour.** Dans l'Inbox, l'utilisateur doit savoir qu'une conversation y est revenue et d'où elle vient, pour décider s'il la remet dans Todo ou Waiting. Les mails déplacés reçoivent, dans la même action, un mot-clé IMAP selon leur dossier d'origine : `$NmWasTodo` ou `$NmWasWaiting`. Le nouveau mail, lui, n'en reçoit pas.

- **Pourquoi un mot-clé IMAP.** La marque est sur le serveur : les autres Mac la reçoivent par la synchronisation ordinaire, sans mécanisme dédié. Elle survit à une perte de la base locale, et le téléphone la montre s'il affiche les mots-clés (Thunderbird oui, Mail sur iOS non). Une colonne locale serait ignorée des autres Mac, et `audit_log` sert à la traçabilité, pas à l'état affiché.
- **Affichage dans la liste.** La liste des conversations de l'Inbox affiche la marque sur chaque conversation revenue, par exemple « ↩ Todo » ou « ↩ Waiting ». C'est une exigence de l'interface, pas une option : sans elle, une conversation revenue ressemble à une conversation nouvelle. Dans les autres dossiers, la marque n'est pas affichée. Le contrat l'expose dans `ThreadSummary` par le champ `returned_from` (`todo`, `waiting` ou absent). Une conversation est marquée si l'un de ses mails rangés dans l'Inbox porte un de ces mots-clés, comme pour le drapeau. Si ses mails portent les deux, ce qui suppose une conversation déjà répartie entre Todo et Waiting, `todo` l'emporte : il reste quelque chose à faire.
- **Retrait.** Par la prochaine commande sur la conversation, quelle qu'elle soit (déplacer, supprimer, marquer comme spam), dans la même action et avec la même source. Une fois que l'utilisateur a décidé, savoir d'où venait la conversation n'a plus d'intérêt. Lire la conversation, changer son drapeau ou ses tags ne retire pas la marque : c'est en lisant que l'utilisateur en a besoin.
- **Mail sorti de l'Inbox ailleurs.** Si la synchronisation constate qu'un mail marqué a quitté l'Inbox par un autre client (téléphone, webmail), le noyau retire le mot-clé (source `core`). Sinon, la marque réapparaîtrait à tort si le mail revenait un jour dans l'Inbox.
- **Retours successifs.** Une conversation ne porte que la marque de son dernier retour : la commande qui l'a remise dans Todo ou Waiting a retiré la précédente.
- **Serveur sans mots-clés personnalisés** (`PERMANENTFLAGS` sans `\*`) : la marque reste locale, dans `message_keyword`, comme les tags. Elle n'est alors ni partagée entre Mac ni visible sur le téléphone.

### Décision sur un expéditeur : rangement des autres mails

Quand l'utilisateur prend ou change une décision sur une adresse, les autres mails déjà reçus de cette adresse sont **systématiquement** rangés selon cette décision. C'est ainsi que l'historique importé se range au fil des décisions.

| Décision | Mails rangés | Depuis |
| --- | --- | --- |
| Inbox | vers l'Inbox | Quarantine seulement |
| Newsletter, Paperwork, Banned | vers le dossier de la décision | Quarantine, Inbox, Archive, et les deux autres dossiers parmi Newsletter, Paperwork et Banned |

- **Une décision Inbox ne vide que la Quarantine.** Les mails déjà archivés ont été traités : les remettre dans l'Inbox (des années d'échanges avec un collègue) serait un retour en arrière.
- **Jamais déplacés** : Todo, Waiting, Sent, Drafts, Deleted et Junk, qui résultent d'une commande explicite de l'utilisateur, ainsi que les mails de l'Inbox marqués d'un drapeau.
- **Décision de domaine** : même règle, pour les mails des adresses du domaine, et de ses sous-domaines (`news.exemple.fr` appartient à `exemple.fr`), qui n'ont pas de décision propre.
- **Authentification** : une décision favorable ne déplace que les mails dont le verdict est accepté, selon l'algorithme de `authentification-expediteur.md`. Un mail usurpé reste en Quarantine. Exception : les mails d'un compte d'archives (import mbox) sont rangés sans condition d'authentification (`import-initial.md`, section 9).
- **Comptes concernés** : tous les comptes, y compris les comptes d'archives, où le déplacement ne modifie que la base locale.
- **Retrait d'une décision** : aucun mail n'est déplacé.
- **Sources** : seules les décisions prises par un client (app, CLI, TUI, web, agent MCP) déclenchent le rangement. Une décision reçue d'un autre Mac (`config_sync`) n'en déclenche pas : cet autre Mac a déjà rangé les mails, et la synchronisation IMAP rapporte ces déplacements. Les décisions prises par le noyau à l'import ne trouvent rien à ranger.
- **Exécution** : un déplacement ordinaire par mail, dans le spool, que l'exécuteur regroupe en `UID MOVE` sur plusieurs UID (`modele de donnees.md`, section 6). Chaque mail déplacé porte la source de la décision (`modified_by`) et une ligne `move` dans `audit_log`.
- **Annulation** : l'app affiche « 342 autres mails rangés · z : annuler ». La touche `z` annule le dernier déplacement ou la dernière décision, puis le précédent, et ainsi de suite (pile de la session). Annuler rétablit la décision précédente (et la politique d'authentification choisie avec une confirmation) et remet chaque mail dans son dossier d'origine. Un changement fait depuis l'emporte : un mail déplacé de nouveau, ou une décision changée de nouveau, n'est pas touché ; annuler deux fois est donc sans effet la seconde fois.
- **Jeton d'annulation** : chaque commande renvoie un jeton, l'intervalle de ses lignes dans `audit_log`. Elles sont consécutives, car la commande tient le verrou d'écriture de sa première écriture à son commit. Annuler rejoue ces lignes à l'envers et écrit ses propres lignes, marquées `undo_of`.

### Authentification de l'expéditeur

L'en-tête `From` est falsifiable : un mail qui se réclame d'un expéditeur approuvé ne doit pas arriver dans l'Inbox s'il n'est pas de lui. Chaque mail reçu porte donc un verdict d'authentification, tiré de l'en-tête `Authentication-Results` du serveur. Une règle Banned s'applique toujours ; une règle favorable (Inbox, Newsletter, Paperwork) n'est appliquée que si le verdict est suffisant, sinon le mail va en Quarantine avec un avertissement. Une règle de domaine favorable exige toujours un mail authentifié.

Le détail (verdicts, confiance au premier usage, politique par adresse, interface) est dans `authentification-expediteur.md`.

### Plusieurs Mac sur une même boîte

Le screening n'a lieu que si NiceMailer tourne sur au moins un Mac : Mac éteint, les mails restent dans l'Inbox du serveur et sont triés au prochain démarrage. Les règles Sieve côté serveur sont écartées.

Quand plusieurs Mac traitent la même boîte (un utilisateur au bureau et à la maison, ou une boîte partagée comme support@), ils partagent les décisions de screening et la configuration de la boîte par un message déposé dans un dossier `nicemailer-sync` du serveur. Cette synchronisation s'active par compte ; la dernière modification l'emporte. Une boîte ne reçoit que les décisions sur des adresses qu'elle a déjà vues. Le détail est dans `synchronisation-configuration.md`.

### Tags

Les tags correspondent aux mots-clés IMAP du serveur et sont décrits par la table `tag` (voir `modele de donnees.md`).