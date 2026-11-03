# NiceMailer : groupes de contacts

Statut : modèle de données validé, rien n'est implémenté. Complète `modele de donnees.md` et `schema.sql`.

## 1. Objet

Certaines applications de mail proposent une liste « VIP » : quelques expéditeurs dont on veut repérer les mails. NiceMailer généralise l'idée : l'utilisateur crée autant de **groupes** qu'il veut (VIP, Famille, Amis, Équipe, Association, Club…) et y range des adresses.

Ce document fixe le modèle de données et sa synchronisation. Les usages (vue d'un groupe, badges, notifications, effet éventuel sur le screening) feront l'objet d'une spécification ultérieure (section 8).

## 2. Principes

- **Un membre est une adresse** (table `address`). Une personne qui écrit depuis deux adresses est ajoutée deux fois. Un domaine entier ne peut pas être membre.
- **Une adresse peut appartenir à plusieurs groupes**, sans restriction : il n'y a pas de groupe exclusif.
- **Les groupes sont communs à tous les comptes**, comme les adresses et les décisions de screening : un collègue est dans « Équipe » quelle que soit la boîte où il écrit.
- **Un groupe ne change rien au tri, pour l'instant.** Appartenir à « VIP » ne fait pas sortir un mail de Quarantine et ne remplace pas une décision de screening. Les deux notions restent séparées : la décision dit *où va* le mail, le groupe dit *qui est* l'expéditeur.
- **Ajouter une adresse à un groupe ne crée pas d'adresse fantôme** : on ajoute une adresse déjà connue (vue dans un mail) ou saisie par l'utilisateur, qui crée alors la ligne `address` avec `created_by` égal à la source du client.

## 3. Modèle de données

Deux tables (`schema.sql`). La première s'appelle `contact_group` et non `group`, mot réservé de SQL qu'il faudrait citer dans chaque requête.

### `contact_group`

| Colonne | Rôle |
| --- | --- |
| `id` | Identifiant local |
| `sync_key` | Identifiant stable entre Mac (`g_` suivi de 16 caractères hexadécimaux tirés au hasard). L'`id` diffère d'un Mac à l'autre, et le nom peut changer : la synchronisation ne peut s'appuyer ni sur l'un ni sur l'autre |
| `name` | Nom affiché, libre. Unique sans casse parmi les groupes non supprimés (index partiel `idx_contact_group_name`) |
| `color` | Couleur d'affichage, NULL pour la couleur par défaut |
| `position` | Ordre d'affichage |
| `date_deleted` | Pierre tombale : un groupe supprimé garde sa ligne et n'est plus affiché |
| Traçabilité | Les quatre colonnes ; `date_modified` suit le nom, la couleur, la position et la suppression |

### `contact_group_member`

| Colonne | Rôle |
| --- | --- |
| `group_id`, `address_id` | Clé primaire : une adresse figure au plus une fois dans un groupe |
| `date_removed` | Pierre tombale : une adresse retirée garde sa ligne |
| Traçabilité | Les quatre colonnes ; `date_modified` suit l'ajout et le retrait |

Index : la clé primaire sert à lister les membres d'un groupe ; `idx_contact_group_member_address` (partiel, membres actifs) sert à trouver les groupes d'une adresse, par exemple pour afficher un badge sur chaque conversation d'une page.

### Règles

- **Supprimer un groupe** pose `date_deleted` ; ses lignes de membres restent (elles ne sont plus lues). Le recréer sous le même nom réutilise la ligne supprimée et ses membres sont retirés (`date_removed`) : un groupe recréé part vide, l'utilisateur n'attend pas le retour des anciens membres.
- **Retirer une adresse** pose `date_removed` ; la remettre efface `date_removed`.
- **Supprimer une adresse** de la base (cas rare, aucune commande ne le fait aujourd'hui) supprime ses appartenances (`ON DELETE CASCADE`).
- Les pierres tombales ne sont jamais purgées, comme celles des tags et des décisions : sans elles, un élément supprimé reviendrait depuis un autre Mac.

## 4. Synchronisation entre Mac

Les groupes et leurs membres sont partagés par la synchronisation de la configuration (`synchronisation-configuration.md`), comme les décisions de screening.

Format, dans le document `nicemailer-config.json` :

```json
"groups": {
  "g_3fa9c2d41b0e7a55": {"name": "VIP",     "color": "#f59e0b", "position": 0, "at": 1760000000},
  "g_8d01e6f2a9c4b310": {"name": null,                                           "at": 1760500000}
},
"group_members": {
  "g_3fa9c2d41b0e7a55": {
    "patron@entreprise.fr": {"member": true,  "at": 1760000100},
    "ancien@entreprise.fr": {"member": false, "at": 1760400000}
  }
}
```

- Un groupe est une entrée, avec un seul horodatage pour le nom, la couleur et la position. `"name": null` est une suppression.
- Une appartenance est une entrée par couple (groupe, adresse). `"member": false` est un retrait.
- Fusion : la règle commune (`synchronisation-configuration.md`, section 5) ; l'entrée la plus récente l'emporte.
- **Deux groupes de même nom** créés sur deux Mac avant synchronisation ont deux `sync_key` différentes. À la réception, si le nom est déjà pris par un autre groupe actif, le groupe reçu est renommé localement « VIP (2) » et l'utilisateur fusionne s'il le souhaite. La fusion automatique est écartée : elle ne serait pas commutative si les deux groupes divergent ensuite.

**Confidentialité** (section 6 du même document) :

- une appartenance n'est écrite dans le document d'une boîte que si l'adresse a été vue dans cette boîte ;
- un groupe n'est écrit que s'il a au moins une appartenance écrite dans ce document, ou s'il y figure déjà. Les collègues d'une boîte partagée ne voient donc pas les noms de groupes sans rapport avec la boîte, comme « Famille ».

## 5. Traçabilité

- `audit_log` : entité `contact_group`, actions `create`, `update` (nom, couleur, position ; avant, après), `delete`, `add_member` et `remove_member` (`detail` : l'adresse).
- Une modification reçue par la synchronisation porte `modified_by = 'config_sync'` et l'horodatage de l'entrée reçue (règle commune).

## 6. Contrat (à préciser à l'implémentation)

Méthodes envisagées : `list_groups` (avec le nombre de membres), `create_group`, `update_group`, `delete_group`, `add_group_member` et `remove_group_member` (par adresse), `list_group_members`. Un événement `GroupChanged {group_id}` signale un groupe ou ses membres modifiés.

## 7. Tests exigés

- Une adresse dans plusieurs groupes ; une adresse ajoutée deux fois au même groupe n'y figure qu'une fois.
- Deux groupes actifs de même nom, à la casse près : refusé. Nom d'un groupe supprimé : réutilisable, la ligne est reprise et le groupe repart vide.
- Retrait puis remise d'un membre : une seule ligne, `date_removed` effacé.
- Synchronisation : suppression d'un groupe et retrait d'un membre propagés par les pierres tombales ; fusion indépendante de l'ordre ; groupe de même nom reçu d'un autre Mac renommé « (2) ».
- Confidentialité : une appartenance d'une adresse jamais vue dans la boîte n'est pas écrite ; un groupe sans appartenance écrite non plus.

## 8. Questions ouvertes

1. **Usages** : vue « VIP » dans la barre latérale (conversations des membres, tous dossiers confondus), badge dans la liste, notifications réservées à certains groupes, filtre de recherche (`group:vip`).
2. **Effet sur le screening** : un membre d'un groupe devrait-il recevoir automatiquement la décision `inbox` ? Pour l'instant non (section 2).
3. **Personnes** : regrouper plusieurs adresses d'une même personne (fiche contact) changerait le membre d'un groupe, de l'adresse à la personne.
4. **Domaines** : permettre « tout `@monentreprise.fr` dans Équipe ».
