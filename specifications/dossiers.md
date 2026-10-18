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

Souvent, lorsqu'on est en mobilité, son ordinateur principal reste allumé à la maison ou au bureau, et niceMailer pourra donc continuer à nettoyer l’inbox, offrant ainsi une expérience de consultation confortable à tout moment.

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

Les mails d'expéditeurs inconnus vont dans le dossier Quarantine où l'utilisateur pourra décider de bannir cet expéditeur (Banned) ou de déplacer le mail vers une cible légitime : Inbox, Newsletter, Paperwork. Le fait de déplacer un mail de Quarantine vers une cible légitime est mémorisé par l’application et les prochains mails de cet expéditeur seront automatiquement livrés dans le dossier approprié.

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

### Données de screening

L'application intègre une table d'adresses (`address`) afin de mémoriser les décisions de screening.
Elle intègre également une table de domaines (`domain`) : l'utilisateur pourra décider pour tous les mails d'un domaine de les classer dans un dossier spécifique. La règle d'adresse l'emporte sur la règle de domaine, et un expéditeur sans règle va en Quarantine.

### Tags

Les tags correspondent aux mots-clés IMAP du serveur et sont décrits par la table `tag` (voir `modele de donnees.md`).