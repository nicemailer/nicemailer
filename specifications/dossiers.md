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
	Tout mails qui a été dans inbox après son traitement : soit une réponse, soit une commande de l'utilisateur
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

J Junk
	Spam
B Banned
	Mails d'expéditeur bloqué par l'utilisateur



## Le fonctionnement des dossiers

NiceMailer classifie les mails entrants en fonction de l'expéditeur.

Les mails d'expéditeurs inconnus vont dans le dossier Quarantine où l'utilisateur pourra décider de bannir cet expéditeur (Banned) ou de déplacer le mail vers une cible légitime : Inbox, Newsletter, Paperwork. Le fait de déplacer un mail de Quarantine vers une cible légitime est mémorisé par l’application et les prochains mails de cet expéditeur seront automatiquement livrés dans le dossier approprié. Les autres mails déjà reçus de cet expéditeur sont rangés en même temps (voir « Décision sur un expéditeur »).

Cela reprend le workflow popularisé par Hey de 37signals.

Les autres dossiers correspondent à des commandes de l'utilisateur sur un mail : Todo, Waiting, Archive, Deleted, Junk. Drafts est alimenté par la rédaction d'un message, et Sent par son envoi.

Pendant la rédaction, le brouillon est enregistré en continu en local (SQLite). Il n'est envoyé au serveur IMAP qu'à la fermeture de la fenêtre de rédaction ou après 20 secondes d'inactivité. IMAP ne permet pas de modifier un message : chaque envoi au serveur est un `APPEND` de la nouvelle version suivi de la suppression de l'ancienne, ce qui change l'UID. Ce délai limite donc le trafic et le nombre de lignes `message` éphémères. Si l'envoi échoue, il est repris via `pending_action` (`append_draft`).

Un mail sorti de Quarantine ne peut pas y retourner.

## Implémentation technique

### A la création du compte IMAP dans niceMailer

Lorsqu'un compte est paramétré dans l'application, après avoir fait une connexion pour tester la validité des paramètres fournis. Nicemailer regarde si les dossiers dont il a besoin sont présents sur le serveur. 

S'ils ne sont pas présents, un panneau s'affiche pour éventuellement sélectionner un dossier déjà présent sur le serveur pour chacun des dossiers du workflow, Drafts compris. Si l'utilisateur n'indique pas un dossier préexistant, les dossiers nécessaires sont créés.

Le panneau propose par défaut les dossiers que le serveur déclare avec un attribut d'usage spécial (RFC 6154 : `\Drafts`, `\Sent`, `\Trash`, `\Junk`, `\Archive`), pour Drafts, Sent, Deleted, Junk et Archive. Un dossier Brouillons déjà utilisé par le téléphone est ainsi repris tel quel, et NiceMailer n'en crée pas un second. À défaut d'attribut, la proposition repose sur le nom du dossier, que l'utilisateur confirme.

Seule l'Inbox est surveillée en IDLE : c'est par elle que les nouveaux mails arrivent sur le serveur, et c'est là que le screening doit réagir sans délai. IDLE ne surveille qu'un dossier par connexion, et multiplier les connexions est coûteux et limité par les serveurs. Les autres dossiers du workflow sont interrogés de temps à autre, toutes les cinq minutes (paramètre `sync.poll_interval_secs`, 300 par défaut), pour récupérer les changements faits ailleurs, par exemple depuis le téléphone. Les actions faites dans NiceMailer sont appliquées localement tout de suite, sans attendre l'interrogation.

### Import initial

À la création du compte, les mails déjà présents sur le serveur ne passent pas par le screening : aucun ne part en Quarantine. L'utilisateur peut archiver les plus anciens, et les personnes à qui il a déjà écrit arrivent directement dans l'Inbox. Les autres expéditeurs passent par la Quarantine à leur prochain mail. Le détail est dans `import-initial.md`.

### Données de screening

L'application intègre une table d'adresses (`address`) afin de mémoriser les décisions de screening.
Elle intègre également une table de domaines (`domain`) : l'utilisateur pourra décider pour tous les mails d'un domaine de les classer dans un dossier spécifique. La règle d'adresse l'emporte sur la règle de domaine, et un expéditeur sans règle va en Quarantine.

### Décision sur un expéditeur : rangement des autres mails

Quand l'utilisateur prend ou change une décision sur une adresse, les autres mails déjà reçus de cette adresse sont **systématiquement** rangés selon cette décision. C'est ainsi que l'historique importé se range au fil des décisions.

| Décision | Mails rangés | Depuis |
| --- | --- | --- |
| Inbox | vers l'Inbox | Quarantine seulement |
| Newsletter, Paperwork, Banned | vers le dossier de la décision | Quarantine, Inbox, Archive, et les deux autres dossiers parmi Newsletter, Paperwork et Banned |

- **Une décision Inbox ne vide que la Quarantine.** Les mails déjà archivés ont été traités : les remettre dans l'Inbox (des années d'échanges avec un collègue) serait un retour en arrière.
- **Jamais déplacés** : Todo, Waiting, Sent, Drafts, Deleted et Junk, qui résultent d'une commande explicite de l'utilisateur, ainsi que les mails de l'Inbox marqués d'un drapeau.
- **Décision de domaine** : même règle, pour les mails des adresses du domaine qui n'ont pas de décision propre.
- **Authentification** : une décision favorable ne déplace que les mails dont le verdict est accepté, selon l'algorithme de `authentification-expediteur.md`. Un mail usurpé reste en Quarantine.
- **Retrait d'une décision** : aucun mail n'est déplacé.
- **Sources** : seules les décisions prises par un client (app, CLI, TUI, web, agent MCP) déclenchent le rangement. Une décision reçue d'un autre Mac (`config_sync`) n'en déclenche pas : cet autre Mac a déjà rangé les mails, et la synchronisation IMAP rapporte ces déplacements. Les décisions prises par le noyau à l'import ne trouvent rien à ranger.
- **Exécution** : un déplacement ordinaire par mail, dans le spool, que l'exécuteur regroupe en `UID MOVE` sur plusieurs UID (`modele de donnees.md`, section 6). Chaque mail déplacé porte la source de la décision (`modified_by`) et une ligne `move` dans `audit_log`.
- **Annulation** : l'app affiche « 342 mails rangés dans Newsletter — Annuler ». Annuler rétablit la décision précédente et remet chaque mail dans son dossier d'origine.

### Authentification de l'expéditeur

L'en-tête `From` est falsifiable : un mail qui se réclame d'un expéditeur approuvé ne doit pas arriver dans l'Inbox s'il n'est pas de lui. Chaque mail reçu porte donc un verdict d'authentification, tiré de l'en-tête `Authentication-Results` du serveur. Une règle Banned s'applique toujours ; une règle favorable (Inbox, Newsletter, Paperwork) n'est appliquée que si le verdict est suffisant, sinon le mail va en Quarantine avec un avertissement. Une règle de domaine favorable exige toujours un mail authentifié.

Le détail (verdicts, confiance au premier usage, politique par adresse, interface) est dans `authentification-expediteur.md`.

### Plusieurs Mac sur une même boîte

Le screening n'a lieu que si NiceMailer tourne sur au moins un Mac : Mac éteint, les mails restent dans l'Inbox du serveur et sont triés au prochain démarrage. Les règles Sieve côté serveur sont écartées.

Quand plusieurs Mac traitent la même boîte (un utilisateur au bureau et à la maison, ou une boîte partagée comme support@), ils partagent les décisions de screening et la configuration de la boîte par un message déposé dans un dossier `nicemailer-sync` du serveur. Cette synchronisation s'active par compte ; la dernière modification l'emporte. Une boîte ne reçoit que les décisions sur des adresses qu'elle a déjà vues. Le détail est dans `synchronisation-configuration.md`.

### Tags

Les tags correspondent aux mots-clés IMAP du serveur et sont décrits par la table `tag` (voir `modele de donnees.md`).