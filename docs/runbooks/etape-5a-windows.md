# Runbook — Étape 5a : aNkou sous Windows

But : sur un PC Windows 10 ou 11 (x64), les mêmes fonctions que sous Linux :
- connexion à un VPS ;
- kill switch, sans fuite DNS ni IPv6 ;
- reconnexion automatique, connexion au démarrage, serveur le plus rapide ;
- application avec icône de notification.

Prérequis :
- les VPS de l'étape 2 en service ;
- un compte (invitation) ;
- un PowerShell **en administrateur** pour l'installation.

## 1. Compiler

Dans `N:\dev\LPDNSoft\ankou`, avec `lpdnsoft-libs` à côté et à jour, et Rust ≥ 1.95 (`rustup update`) :

```powershell
cargo build --release -p ankoud -p ankou-cli -p ankou
```

## 2. Installer (PowerShell administrateur)

```powershell
$dest = "C:\Program Files\aNkou"
New-Item -ItemType Directory -Force "$dest\resources-windows\binaries" | Out-Null
Copy-Item target\release\ankoud.exe, target\release\ankou-cli.exe, target\release\ankou.exe $dest

# wireguard.dll, signée par WireGuard LLC
Invoke-WebRequest https://download.wireguard.com/wireguard-nt/wireguard-nt-0.10.1.zip -OutFile "$env:TEMP\wgnt.zip"
Expand-Archive -Force "$env:TEMP\wgnt.zip" "$env:TEMP\wgnt"
Get-AuthenticodeSignature "$env:TEMP\wgnt\wireguard-nt\bin\amd64\wireguard.dll" | Format-List Status, SignerCertificate
Copy-Item "$env:TEMP\wgnt\wireguard-nt\bin\amd64\wireguard.dll" "$dest\resources-windows\binaries\wireguard-amd64.dll"

# Service, démarré avec Windows et relancé s'il s'arrête
sc.exe create ankoud binPath= "\"$dest\ankoud.exe\" --service" start= auto DisplayName= "aNkou VPN"
sc.exe failure ankoud reset= 0 actions= restart/5000
sc.exe start ankoud

# Commandes accessibles partout (pour la session en cours ; ajoutez-le au PATH pour la suite)
$env:Path += ";$dest"
ankou-cli status
```

La signature doit afficher `Status : Valid` et `CN=WireGuard LLC`. `ankou-cli status` doit afficher « Déconnecté. » puis « Kill switch : activé ».

Ensuite :
- créez un raccourci de `C:\Program Files\aNkou\ankou.exe` sur le bureau ou dans le menu Démarrer ;
- pour la lancer à l'ouverture de session, placez ce raccourci dans `shell:startup`.

## 3. Première connexion

1. Ouvrez **aNkou**, onglet **Compte** :
   - adresse de l'API (`https://api.<votre-domaine>`) ;
   - e-mail, mot de passe, puis **Se connecter au compte**. Pour un compte neuf : « Pas encore de compte ? », puis le code d'invitation.
2. Indiquez le nom de l'appareil (par ex. « PC-Windows »), puis **Enrôler cet appareil**.
3. L'accueil passe à « Connecté à … ».

En ligne de commande, les mêmes opérations : `ankou-cli login`, `ankou-cli enroll --name PC-Windows --out C:\Users\<vous>\ankou.toml`, puis `ankou-cli connect --config …`.

## 4. Tests d'acceptation (critère de fin de l'étape 5a)

Connecté, kill switch activé :

1. **IP** : https://ifconfig.me affiche l'IP du VPS.
2. **DNS** : https://dnsleaktest.com, test étendu : seuls des serveurs Quad9 apparaissent.
3. **IPv6** : `curl.exe -6 --max-time 5 https://ifconfig.co` échoue.
4. **Kill switch** :
   - `Disable-NetAdapter -Name ankou0 -Confirm:$false`, en administrateur ;
   - aussitôt, `curl.exe -4 --max-time 5 https://ifconfig.me` doit **échouer** ;
   - le service se reconnecte seul en 5 s au plus, puis la même commande affiche de nouveau l'IP du VPS.
5. **Plantage du service** : `taskkill /F /IM ankoud.exe`.
   - Internet reste bloqué hors tunnel (filtres persistants).
   - Windows relance le service (5 s), qui se reconnecte.
6. **Redémarrage du PC** pendant une connexion : au retour, aNkou se reconnecte, sans fuite entre-temps.
7. **Changement de réseau** (Wi-Fi vers partage de connexion du téléphone) : reconnexion seule.
8. **Serveur le plus rapide** : déconnecté, **Serveur le plus rapide** affiche « … (xx ms) », puis se connecte.
9. **Icône** :
   - sa couleur suit l'état ;
   - le menu (clic droit) propose Se connecter / Se déconnecter / Serveur le plus rapide / Ouvrir / Quitter ;
   - fermer la fenêtre la cache, et un clic sur l'icône la rouvre.
10. **Déconnexion** : **Se déconnecter**, puis votre IP habituelle revient et Internet fonctionne normalement.

L'étape 5a est terminée quand ces tests passent.

## 5. En cas de problème

- **« WireGuard n'est pas disponible (wireguard.dll introuvable…) »** : vérifiez `C:\Program Files\aNkou\resources-windows\binaries\wireguard-amd64.dll` (section 2).
- **« Le service ankoud ne tourne pas »** : `sc.exe start ankoud` (administrateur). Journaux : `C:\ProgramData\aNkou\logs`.
- **Plus d'Internet** : c'est le kill switch qui bloque. Faites `ankou-cli disconnect` ; ou bien `ankou-cli killswitch off` pour rester connecté sans blocage.
- **Dernier recours** (service supprimé, filtres restés), en administrateur :
  ```powershell
  sc.exe stop ankoud
  & "C:\Program Files\aNkou\ankoud.exe" --state-dir $env:TEMP\ankou-reset
  ```
  Sans profil dans ce dossier vide, `ankoud` retire les filtres restés dès son démarrage. Faites ensuite Ctrl+C.
  Pour voir les filtres : `netsh wfp show filters`, qui écrit `filters.xml` (fournisseur « aNkou »).
- **Désinstaller** :
  ```powershell
  ankou-cli disconnect
  sc.exe stop ankoud
  sc.exe delete ankoud
  ```
  Puis supprimez `C:\Program Files\aNkou` et `C:\ProgramData\aNkou`.

## 6. Limites connues

- L'installeur MSI arrive à l'étape 7 : en attendant, installation par `sc.exe` (section 2).
- Pas de signature de code : Windows SmartScreen peut avertir au premier lancement de `ankou.exe`.
- Mesure du serveur le plus rapide pendant une connexion : elle passe par le tunnel courant (ADR 0009) ; déconnecté, elle est exacte.
- Le réseau local (imprimante, NAS) n'est pas joignable avec le kill switch, comme sous Linux.
