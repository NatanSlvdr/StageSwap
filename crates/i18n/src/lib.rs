use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Locale {
    #[default]
    English,
    French,
    Spanish,
}

impl Locale {
    pub const ALL: [Self; 3] = [Self::English, Self::French, Self::Spanish];

    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::French => "fr-FR",
            Self::Spanish => "es",
        }
    }

    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
            Self::Spanish => "Español",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        let language = tag
            .trim()
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match language.as_str() {
            "en" => Some(Self::English),
            "fr" => Some(Self::French),
            "es" => Some(Self::Spanish),
            _ => None,
        }
    }

    pub fn resolve(saved: Option<&str>, system: Option<&str>) -> Self {
        saved
            .and_then(Self::from_tag)
            .or_else(|| system.and_then(Self::from_tag))
            .unwrap_or_default()
    }
}

macro_rules! catalog {
    ($locale:expr, $source:expr, { $($english:literal => ($french:literal, $spanish:literal)),* $(,)? }) => {
        match $source {
            $(
                $english => match $locale {
                    Locale::English => $english,
                    Locale::French => $french,
                    Locale::Spanish => $spanish,
                },
            )*
            other => other,
        }
    };
}

pub fn text<'a>(locale: Locale, source: &'a str) -> Cow<'a, str> {
    Cow::Borrowed(catalog!(locale, source, {
        // Common controls and states.
        "On" => ("Activé", "Activado"),
        "Off" => ("Désactivé", "Desactivado"),
        "Ready" => ("Prêt", "Listo"),
        "Missing" => ("Manquante", "Falta"),
        "Waiting" => ("En attente", "Esperando"),
        "Matching" => ("Correspondance", "Coincidencia"),
        "Not matching" => ("Aucune correspondance", "No coincide"),
        "Reference missing" => ("Référence manquante", "Falta la referencia"),
        "Initializing" => ("Initialisation", "Inicializando"),
        "Unavailable" => ("Indisponible", "No disponible"),
        "Failed" => ("Échec", "Error"),
        "Automatic" => ("Automatique", "Automático"),
        "Camera" => ("Caméra", "Cámara"),
        "Display" => ("Écran", "Pantalla"),
        "Webcam" => ("Webcam", "Cámara web"),
        "Screen" => ("Écran", "Pantalla"),
        "Output" => ("Sortie", "Salida"),
        "Reference" => ("Référence", "Referencia"),
        "Detection" => ("Détection", "Detección"),
        "Live" => ("EN DIRECT", "EN DIRECTO"),
        "LIVE" => ("EN DIRECT", "EN DIRECTO"),
        "WEBCAM" => ("WEBCAM", "CÁMARA WEB"),
        "SCREEN" => ("ÉCRAN", "PANTALLA"),
        "REFERENCE" => ("RÉFÉRENCE", "REFERENCIA"),
        "OUTPUT" => ("SORTIE", "SALIDA"),
        "Unknown" => ("Inconnu", "Desconocido"),
        "Crossfading" => ("Fondu enchaîné", "Fundido cruzado"),
        "Screen mix" => ("Mixage écran", "Mezcla de pantalla"),
        "Auto" => ("Auto", "Auto"),
        "Close" => ("Fermer", "Cerrar"),
        "Cancel" => ("Annuler", "Cancelar"),
        "Back" => ("Retour", "Atrás"),
        "Continue" => ("Continuer", "Continuar"),
        "Skip" => ("Ignorer", "Omitir"),

        // Dashboard and previews.
        "Settings" => ("Paramètres", "Ajustes"),
        "Components health" => ("État des composants", "Estado de los componentes"),
        "Main controls" => ("Commandes principales", "Controles principales"),
        "Start automation" => ("Démarrer l’automatisation", "Iniciar automatización"),
        "Stop automation" => ("Arrêter l’automatisation", "Detener automatización"),
        "Stopping automation…" => ("Arrêt de l’automatisation…", "Deteniendo automatización…"),
        "Output mode" => ("Mode de sortie", "Modo de salida"),
        "Other" => ("Autres actions", "Otras acciones"),
        "Capture reference" => ("Capturer la référence", "Capturar referencia"),
        "Rescan screens" => ("Rechercher les écrans", "Buscar pantallas"),
        "No webcam frame" => ("Aucune image de webcam", "No hay imagen de la cámara web"),
        "No screen frame" => ("Aucune image d’écran", "No hay imagen de pantalla"),
        "No reference image" => ("Aucune image de référence", "No hay imagen de referencia"),
        "No output frame" => ("Aucune image de sortie", "No hay imagen de salida"),
        "Preparing preview…" => ("Préparation de l’aperçu…", "Preparando vista previa…"),

        // Settings shell.
        "General" => ("Général", "General"),
        "Diagnostics" => ("Diagnostic", "Diagnóstico"),
        "Choose how the app launches, stays available, and reports problems." => ("Choisissez le mode de démarrage de l’application, sa disponibilité et le signalement des problèmes.", "Elige cómo se inicia la aplicación, cómo permanece disponible y cómo informa de los problemas."),
        "Select, verify, and recover the camera used for webcam output." => ("Sélectionnez, vérifiez et rétablissez la caméra utilisée pour la sortie webcam.", "Selecciona, comprueba y recupera la cámara utilizada para la salida de cámara web."),
        "Choose the display Automatic mode watches and how it is captured." => ("Choisissez l’écran surveillé par le mode Automatique et la façon dont il est capturé.", "Elige la pantalla que vigila el modo Automático y cómo se captura."),
        "Teach Automatic mode when the screen should show the webcam." => ("Indiquez au mode Automatique quand l’écran doit afficher la webcam.", "Indica al modo Automático cuándo debe mostrarse la cámara web."),
        "Inspect component health, technical details, logs, and recovery tools." => ("Consultez l’état des composants, les détails techniques, les journaux et les outils de récupération.", "Consulta el estado de los componentes, los detalles técnicos, los registros y las herramientas de recuperación."),
        "PREFERENCES" => ("PRÉFÉRENCES", "PREFERENCIAS"),
        "Back to dashboard" => ("Retour au tableau de bord", "Volver al panel"),
        "AUTOSAVE" => ("ENREGISTREMENT AUTO.", "GUARDADO AUTOMÁTICO"),
        "Saved" => ("Enregistré", "Guardado"),
        "Saving…" => ("Enregistrement…", "Guardando…"),
        "Couldn’t save" => ("Échec de l’enregistrement", "No se pudo guardar"),

        // General settings.
        "How StageSwap works" => ("Fonctionnement de StageSwap", "Cómo funciona StageSwap"),
        "StageSwap watches your selected screen. While it matches your saved reference image, your video calls see your webcam. When the screen changes, StageSwap automatically switches to the screen. When the reference returns, it switches back to your webcam." => ("StageSwap surveille l’écran sélectionné. Tant qu’il correspond à l’image de référence enregistrée, vos appels vidéo voient votre webcam. Lorsque l’écran change, StageSwap bascule automatiquement vers l’écran. Lorsque la référence réapparaît, il revient à votre webcam.", "StageSwap vigila la pantalla seleccionada. Mientras coincida con la imagen de referencia guardada, las videollamadas verán tu cámara web. Cuando cambie la pantalla, StageSwap cambiará automáticamente a ella. Cuando vuelva la referencia, regresará a la cámara web."),
        "Dashboard tutorial" => ("Tutoriel du tableau de bord", "Tutorial del panel"),
        "Review the previews, status information, and everyday controls." => ("Découvrez les aperçus, les informations d’état et les commandes courantes.", "Repasa las vistas previas, la información de estado y los controles habituales."),
        "Open tutorial" => ("Ouvrir le tutoriel", "Abrir tutorial"),
        "Startup" => ("Démarrage", "Inicio"),
        "Applied the next time StageSwap starts." => ("Appliqué au prochain démarrage de StageSwap.", "Se aplica la próxima vez que se inicie StageSwap."),
        "Start with Windows" => ("Démarrer avec Windows", "Iniciar con Windows"),
        "Install StageSwap to use a stable Windows startup path." => ("Installez StageSwap pour utiliser un chemin de démarrage Windows stable.", "Instala StageSwap para usar una ruta de inicio de Windows estable."),
        "Install StageSwap to enable startup" => ("Installer StageSwap pour activer le démarrage", "Instalar StageSwap para activar el inicio"),
        "Launch after Windows sign-in." => ("Lancer après la connexion à Windows.", "Iniciar después de entrar en Windows."),
        "Start minimized" => ("Démarrer réduit", "Iniciar minimizado"),
        "Open in the system tray." => ("Ouvrir dans la zone de notification.", "Abrir en la bandeja del sistema."),
        "Start automation on launch" => ("Démarrer l’automatisation au lancement", "Iniciar la automatización al abrir"),
        "Window behavior" => ("Comportement de la fenêtre", "Comportamiento de la ventana"),
        "Choose what closing StageSwap does." => ("Choisissez l’action effectuée à la fermeture de StageSwap.", "Elige qué ocurre al cerrar StageSwap."),
        "Close window to tray" => ("Réduire la fenêtre dans la zone de notification", "Cerrar la ventana en la bandeja"),
        "Keep StageSwap running after closing the window." => ("Laisser StageSwap actif après la fermeture de la fenêtre.", "Mantener StageSwap en ejecución después de cerrar la ventana."),
        "Confirm before exit" => ("Confirmer avant de quitter", "Confirmar antes de salir"),
        "Ask before StageSwap fully exits." => ("Demander confirmation avant de quitter complètement StageSwap.", "Preguntar antes de cerrar StageSwap por completo."),
        "Notifications" => ("Notifications", "Notificaciones"),
        "Important Windows warnings." => ("Alertes Windows importantes.", "Avisos importantes de Windows."),
        "Show status notifications" => ("Afficher les notifications d’état", "Mostrar notificaciones de estado"),
        "Notify when a component needs attention." => ("Notifier lorsqu’un composant nécessite votre attention.", "Avisar cuando un componente requiera atención."),
        "Language" => ("Langue", "Idioma"),
        "Choose the language used by StageSwap." => ("Choisissez la langue utilisée par StageSwap.", "Elige el idioma que utiliza StageSwap."),
        "Interface language" => ("Langue de l’interface", "Idioma de la interfaz"),
        "Changes apply immediately." => ("Les modifications s’appliquent immédiatement.", "Los cambios se aplican de inmediato."),
        "On — Starts in {0} mode {1}." => ("Activé — Démarre en mode {0} {1}.", "Activado: se inicia en modo {0} {1}."),
        "in the tray" => ("dans la zone de notification", "en la bandeja"),
        "after the dashboard opens" => ("après l’ouverture du tableau de bord", "después de abrir el panel"),
        "Off — Shows the StageSwap off screen until automation starts." => ("Désactivé — Affiche l’écran d’arrêt de StageSwap jusqu’au démarrage de l’automatisation.", "Desactivado: muestra la pantalla de StageSwap desactivado hasta que se inicie la automatización."),
        "Closing hides the window; Exit from the tray asks for confirmation." => ("La fermeture masque la fenêtre ; Quitter depuis la zone de notification demande confirmation.", "Al cerrar se oculta la ventana; Salir desde la bandeja pide confirmación."),
        "Closing hides the window; Exit from the tray stops StageSwap immediately." => ("La fermeture masque la fenêtre ; Quitter depuis la zone de notification arrête immédiatement StageSwap.", "Al cerrar se oculta la ventana; Salir desde la bandeja detiene StageSwap de inmediato."),
        "Closing the window or choosing Exit asks before StageSwap stops." => ("Fermer la fenêtre ou choisir Quitter demande confirmation avant l’arrêt de StageSwap.", "Cerrar la ventana o elegir Salir pide confirmación antes de detener StageSwap."),
        "Closing the window or choosing Exit stops StageSwap immediately." => ("Fermer la fenêtre ou choisir Quitter arrête immédiatement StageSwap.", "Cerrar la ventana o elegir Salir detiene StageSwap de inmediato."),

        // Webcam and screen settings.
        "No camera selected" => ("Aucune caméra sélectionnée", "No hay ninguna cámara seleccionada"),
        "Saved camera is unavailable" => ("La caméra enregistrée est indisponible", "La cámara guardada no está disponible"),
        "Camera input" => ("Entrée caméra", "Entrada de cámara"),
        "Used by Camera mode and whenever Automatic selects Camera. Output is always 16:9." => ("Utilisée par le mode Caméra et lorsque le mode Automatique sélectionne la caméra. La sortie est toujours au format 16:9.", "Se utiliza en el modo Cámara y cuando Automático selecciona Cámara. La salida siempre es 16:9."),
        "Selected webcam" => ("Webcam sélectionnée", "Cámara web seleccionada"),
        "No webcam frame — choose a camera or refresh the device list." => ("Aucune image de webcam — choisissez une caméra ou actualisez la liste des appareils.", "No hay imagen de la cámara web: elige una cámara o actualiza la lista de dispositivos."),
        "Refresh camera devices" => ("Actualiser les caméras", "Actualizar cámaras"),
        "Crop webcam to 16:9" => ("Recadrer la webcam en 16:9", "Recortar la cámara web a 16:9"),
        "Crop non-16:9 cameras to fill the frame." => ("Recadrer les caméras qui ne sont pas en 16:9 afin de remplir l’image.", "Recortar las cámaras que no sean 16:9 para llenar el encuadre."),
        "No display selected" => ("Aucun écran sélectionné", "No hay ninguna pantalla seleccionada"),
        "Screen capture" => ("Capture d’écran", "Captura de pantalla"),
        "This display is used by Display mode and watched by Automatic mode for reference changes." => ("Cet écran est utilisé par le mode Écran et surveillé par le mode Automatique pour détecter les changements de référence.", "Esta pantalla se utiliza en el modo Pantalla y el modo Automático la vigila para detectar cambios en la referencia."),
        "Live screen" => ("Écran en direct", "Pantalla en directo"),
        "No screen frame — choose a display or use Recovery in Diagnostics." => ("Aucune image d’écran — choisissez un écran ou utilisez Récupération dans Diagnostic.", "No hay imagen de pantalla: elige una pantalla o usa Recuperación en Diagnóstico."),
        "Capture" => ("Capture", "Captura"),
        "Capture behavior" => ("Comportement de la capture", "Comportamiento de la captura"),
        "Include mouse cursor" => ("Inclure le pointeur de la souris", "Incluir el cursor del ratón"),
        "New references follow this choice; existing and imported references do not change." => ("Les nouvelles références suivent ce choix ; les références existantes et importées ne changent pas.", "Las referencias nuevas siguen esta opción; las existentes e importadas no cambian."),
        "Automatic discovery and recovery" => ("Découverte et récupération automatiques", "Detección y recuperación automáticas"),
        "Find reference display automatically" => ("Trouver automatiquement l’écran de référence", "Buscar automáticamente la pantalla de referencia"),
        "Recover black screen capture automatically" => ("Récupérer automatiquement une capture noire", "Recuperar automáticamente una captura en negro"),
        "On — Searches at launch, Settings open, reference changes, and every 30 seconds; confirms the same display twice." => ("Activé — Recherche au démarrage, à l’ouverture des Paramètres, lors des changements de référence et toutes les 30 secondes ; confirme deux fois le même écran.", "Activado: busca al iniciar, al abrir Ajustes, cuando cambia la referencia y cada 30 segundos; confirma dos veces la misma pantalla."),
        "Off — Choose a display manually or use Rescan displays." => ("Désactivé — Choisissez un écran manuellement ou utilisez Rechercher les écrans.", "Desactivado: elige una pantalla manualmente o usa Buscar pantallas."),
        "On — Checks the selected display every 30 seconds and restarts after two black results. Black content can trigger recovery." => ("Activé — Vérifie l’écran sélectionné toutes les 30 secondes et redémarre après deux résultats noirs. Un contenu noir peut déclencher la récupération.", "Activado: comprueba la pantalla seleccionada cada 30 segundos y reinicia tras dos resultados en negro. El contenido negro puede activar la recuperación."),
        "Off — Use Restart screen capture in Diagnostics." => ("Désactivé — Utilisez Redémarrer la capture d’écran dans Diagnostic.", "Desactivado: usa Reiniciar captura de pantalla en Diagnóstico."),

        // Matching.
        "Reference matching" => ("Correspondance de référence", "Coincidencia de referencia"),
        "Reference matches → Camera. Reference changes → Display. Without a usable reference, Automatic mode stays on Camera." => ("Référence identique → Caméra. Référence modifiée → Écran. Sans référence utilisable, le mode Automatique reste sur Caméra.", "La referencia coincide → Cámara. La referencia cambia → Pantalla. Sin una referencia válida, el modo Automático permanece en Cámara."),
        "Reference matches: Camera. Reference changes: Display. Without a usable reference, Automatic mode stays on Camera." => ("Référence identique : Caméra. Référence modifiée : Écran. Sans référence utilisable, le mode Automatique reste sur Caméra.", "La referencia coincide: Cámara. La referencia cambia: Pantalla. Sin una referencia válida, el modo Automático permanece en Cámara."),
        "Reference image" => ("Image de référence", "Imagen de referencia"),
        "No reference image — capture the current screen or import one." => ("Aucune image de référence — capturez l’écran actuel ou importez-en une.", "No hay imagen de referencia: captura la pantalla actual o importa una."),
        "Checks 4×/s · 5 matches or 3 mismatches · 0.5s fade" => ("4 vérifications/s · 5 correspondances ou 3 différences · fondu de 0,5 s", "4 comprobaciones/s · 5 coincidencias o 3 diferencias · fundido de 0,5 s"),
        "Capture screen" => ("Capturer l’écran", "Capturar pantalla"),
        "Import image…" => ("Importer une image…", "Importar imagen…"),
        "Match strictness" => ("Précision de la correspondance", "Precisión de coincidencia"),
        "Reset 98%" => ("Réinitialiser à 98 %", "Restablecer al 98 %"),
        "Very strict" => ("Très stricte", "Muy estricta"),
        "Balanced" => ("Équilibrée", "Equilibrada"),
        "Forgiving" => ("Tolérante", "Flexible"),
        "Very forgiving" => ("Très tolérante", "Muy flexible"),
        "Small visual changes can switch Automatic mode to Display." => ("De petits changements visuels peuvent faire basculer le mode Automatique vers Écran.", "Los pequeños cambios visuales pueden hacer que el modo Automático cambie a Pantalla."),
        "Minor rendering or cursor differences can still match the reference." => ("De légères différences de rendu ou de pointeur peuvent toujours correspondre à la référence.", "Las pequeñas diferencias de renderizado o del cursor aún pueden coincidir con la referencia."),
        "Larger differences may still count as the reference." => ("Des différences plus importantes peuvent toujours être considérées comme la référence.", "Las diferencias mayores aún pueden contar como referencia."),
        "Meaningful changes may still be treated as a match." => ("Des changements significatifs peuvent toujours être considérés comme une correspondance.", "Los cambios importantes aún pueden tratarse como coincidencias."),

        // Diagnostics.
        "Component health" => ("État des composants", "Estado de los componentes"),
        "Current pipeline state." => ("État actuel du pipeline.", "Estado actual del flujo."),
        "Virtual camera" => ("Caméra virtuelle", "Cámara virtual"),
        "Recovery" => ("Récupération", "Recuperación"),
        "Rescan finds the reference display. Restart buttons reconnect only the named component." => ("La recherche retrouve l’écran de référence. Les boutons de redémarrage reconnectent uniquement le composant indiqué.", "La búsqueda encuentra la pantalla de referencia. Los botones de reinicio solo vuelven a conectar el componente indicado."),
        "Rescan displays" => ("Rechercher les écrans", "Buscar pantallas"),
        "Restart webcam" => ("Redémarrer la webcam", "Reiniciar cámara web"),
        "Restart screen capture" => ("Redémarrer la capture d’écran", "Reiniciar captura de pantalla"),
        "Restart virtual camera" => ("Redémarrer la caméra virtuelle", "Reiniciar cámara virtual"),
        "Restart all" => ("Tout redémarrer", "Reiniciar todo"),
        "Technical details" => ("Détails techniques", "Detalles técnicos"),
        "Identifiers, formats, and timing used by the active pipeline." => ("Identifiants, formats et temporisation utilisés par le pipeline actif.", "Identificadores, formatos y tiempos utilizados por el flujo activo."),
        "Webcam device ID" => ("Identifiant de la webcam", "ID de la cámara web"),
        "Webcam format" => ("Format de la webcam", "Formato de la cámara web"),
        "Output pipeline" => ("Pipeline de sortie", "Flujo de salida"),
        "Transitions" => ("Transitions", "Transiciones"),
        "Detection timing" => ("Temporisation de la détection", "Frecuencia de detección"),
        "Storage and logs" => ("Stockage et journaux", "Almacenamiento y registros"),
        "Settings, references, and 14-day logs stay on this computer." => ("Les paramètres, les références et les journaux conservés 14 jours restent sur cet ordinateur.", "Los ajustes, las referencias y los registros de 14 días permanecen en este equipo."),
        "Configuration" => ("Configuration", "Configuración"),
        "Log directory" => ("Dossier des journaux", "Carpeta de registros"),
        "Diagnostic logs" => ("Journaux de diagnostic", "Registros de diagnóstico"),
        "Stored locally for troubleshooting." => ("Stockés localement pour le dépannage.", "Se guardan localmente para solucionar problemas."),
        "Logs are retained for 14 days." => ("Les journaux sont conservés pendant 14 jours.", "Los registros se conservan durante 14 días."),
        "Open log folder" => ("Ouvrir le dossier des journaux", "Abrir carpeta de registros"),
        "Export logs…" => ("Exporter les journaux…", "Exportar registros…"),
        "Clear logs…" => ("Effacer les journaux…", "Borrar registros…"),
        "Open folder" => ("Ouvrir le dossier", "Abrir carpeta"),
        "Export…" => ("Exporter…", "Exportar…"),
        "Clear…" => ("Effacer…", "Borrar…"),
        "The webcam needs attention. Choose or refresh the camera in Webcam, then restart it here if needed." => ("La webcam nécessite votre attention. Choisissez ou actualisez la caméra dans Webcam, puis redémarrez-la ici si nécessaire.", "La cámara web requiere atención. Elige o actualiza la cámara en Cámara web y reiníciala aquí si es necesario."),
        "Screen capture needs attention. Choose a display in Screen, then restart capture here if needed." => ("La capture d’écran nécessite votre attention. Choisissez un écran dans Écran, puis redémarrez la capture ici si nécessaire.", "La captura de pantalla requiere atención. Elige una pantalla en Pantalla y reinicia la captura aquí si es necesario."),
        "The virtual camera needs attention. Restart it here, then reselect StageSwap in the meeting app if necessary." => ("La caméra virtuelle nécessite votre attention. Redémarrez-la ici, puis sélectionnez de nouveau StageSwap dans l’application de réunion si nécessaire.", "La cámara virtual requiere atención. Reiníciala aquí y vuelve a seleccionar StageSwap en la aplicación de reuniones si es necesario."),
        "One or more components are still starting. Wait briefly before using a recovery action." => ("Un ou plusieurs composants sont encore en cours de démarrage. Patientez un instant avant d’utiliser une action de récupération.", "Uno o varios componentes todavía se están iniciando. Espera un momento antes de usar una acción de recuperación."),
        "The pipeline is ready, but Automatic mode needs a captured or imported reference." => ("Le pipeline est prêt, mais le mode Automatique nécessite une référence capturée ou importée.", "El flujo está listo, pero el modo Automático necesita una referencia capturada o importada."),
        "The components are ready; StageSwap is waiting for enough reference checks to decide." => ("Les composants sont prêts ; StageSwap attend suffisamment de vérifications de la référence pour prendre une décision.", "Los componentes están listos; StageSwap espera suficientes comprobaciones de la referencia para decidir."),
        "Everything is ready." => ("Tout est prêt.", "Todo está listo."),
        "Admin configuration" => ("Configuration administrateur", "Configuración de administrador"),

        // Dialogs.
        "Exit StageSwap?" => ("Quitter StageSwap ?", "¿Salir de StageSwap?"),
        "Clear diagnostic logs?" => ("Effacer les journaux de diagnostic ?", "¿Borrar los registros de diagnóstico?"),
        "Replace saved configuration?" => ("Remplacer la configuration enregistrée ?", "¿Sustituir la configuración guardada?"),
        "Load saved configuration?" => ("Charger la configuration enregistrée ?", "¿Cargar la configuración guardada?"),
        "Delete saved configuration?" => ("Supprimer la configuration enregistrée ?", "¿Eliminar la configuración guardada?"),
        "StageSwap will stop publishing. The virtual camera stays installed and shows the StageSwap off screen until the app starts again." => ("StageSwap cessera de diffuser. La caméra virtuelle reste installée et affiche l’écran d’arrêt de StageSwap jusqu’au prochain démarrage de l’application.", "StageSwap dejará de emitir. La cámara virtual seguirá instalada y mostrará la pantalla de StageSwap desactivado hasta que la aplicación vuelva a iniciarse."),
        "This permanently removes locally stored diagnostic logs. New logs will continue to be recorded." => ("Cette action supprime définitivement les journaux de diagnostic stockés localement. De nouveaux journaux continueront d’être enregistrés.", "Esto elimina de forma permanente los registros de diagnóstico guardados localmente. Se seguirán registrando nuevos datos."),
        "Replace the saved admin config with the setup currently shown in Settings?" => ("Remplacer la configuration administrateur enregistrée par celle actuellement affichée dans Paramètres ?", "¿Sustituir la configuración de administrador guardada por la que se muestra actualmente en Ajustes?"),
        "Replace the current settings and reference image with the saved admin config? Current session changes will be lost." => ("Remplacer les paramètres actuels et l’image de référence par la configuration administrateur enregistrée ? Les modifications de la session seront perdues.", "¿Sustituir los ajustes actuales y la imagen de referencia por la configuración de administrador guardada? Se perderán los cambios de la sesión."),
        "Auto-restore will turn off. Your current settings and reference image will stay unchanged." => ("La restauration automatique sera désactivée. Vos paramètres et votre image de référence actuels resteront inchangés.", "La restauración automática se desactivará. Los ajustes actuales y la imagen de referencia no cambiarán."),
        "Stay open" => ("Rester ouvert", "Mantener abierto"),
        "Exit StageSwap" => ("Quitter StageSwap", "Salir de StageSwap"),
        "Keep logs" => ("Conserver les journaux", "Conservar registros"),
        "Clear logs" => ("Effacer les journaux", "Borrar registros"),
        "Keep saved configuration" => ("Conserver la configuration enregistrée", "Conservar configuración guardada"),
        "Save current configuration" => ("Enregistrer la configuration actuelle", "Guardar configuración actual"),
        "Keep current config" => ("Conserver la configuration actuelle", "Conservar configuración actual"),
        "Load saved configuration" => ("Charger la configuration enregistrée", "Cargar configuración guardada"),
        "Delete saved configuration" => ("Supprimer la configuration enregistrée", "Eliminar configuración guardada"),
        "Keep a protected local copy of the current settings and reference image for managed setups." => ("Conservez une copie locale protégée des paramètres actuels et de l’image de référence pour les installations gérées.", "Conserva una copia local protegida de los ajustes actuales y la imagen de referencia para instalaciones administradas."),
        "No admin config is saved." => ("Aucune configuration administrateur n’est enregistrée.", "No hay ninguna configuración de administrador guardada."),
        "Settings and reference image saved in the admin config" => ("Paramètres et image de référence enregistrés dans la configuration administrateur", "Ajustes e imagen de referencia guardados en la configuración de administrador"),
        "Admin config saved without a reference image" => ("Configuration administrateur enregistrée sans image de référence", "Configuración de administrador guardada sin imagen de referencia"),
        "Auto-restore on launch" => ("Restaurer automatiquement au démarrage", "Restaurar automáticamente al iniciar"),
        "Replace session changes with this admin config whenever StageSwap starts." => ("Remplacer les modifications de la session par cette configuration administrateur à chaque démarrage de StageSwap.", "Sustituir los cambios de la sesión por esta configuración de administrador cada vez que se inicie StageSwap."),

        // First-run and dashboard tutorial.
        "STEP {0} OF {1}" => ("ÉTAPE {0} SUR {1}", "PASO {0} DE {1}"),
        "Skip tutorial" => ("Ignorer le tutoriel", "Omitir tutorial"),
        "Finish" => ("Terminer", "Finalizar"),
        "Next" => ("Suivant", "Siguiente"),
        "Welcome to StageSwap" => ("Bienvenue dans StageSwap", "Te damos la bienvenida a StageSwap"),
        "Automatic chooses for you" => ("Le mode Automatique choisit pour vous", "Automático elige por ti"),
        "Know the four previews" => ("Découvrez les quatre aperçus", "Conoce las cuatro vistas previas"),
        "Check what is ready" => ("Vérifiez ce qui est prêt", "Comprueba qué está listo"),
        "Control what people see" => ("Contrôlez ce que les autres voient", "Controla lo que ven los demás"),
        "Prepare the screen" => ("Préparez l’écran", "Prepara la pantalla"),
        "Ready to use StageSwap" => ("Prêt à utiliser StageSwap", "Todo listo para usar StageSwap"),
        "Choose StageSwap in Zoom" => ("Choisissez StageSwap dans Zoom", "Elige StageSwap en Zoom"),
        "StageSwap creates a virtual camera for Zoom. Open Zoom’s camera list and select StageSwap instead of your physical webcam." => ("StageSwap crée une caméra virtuelle pour Zoom. Ouvrez la liste des caméras de Zoom et sélectionnez StageSwap à la place de votre webcam physique.", "StageSwap crea una cámara virtual para Zoom. Abre la lista de cámaras de Zoom y selecciona StageSwap en lugar de la cámara web física."),
        "One camera, two possible views" => ("Une caméra, deux vues possibles", "Una cámara, dos vistas posibles"),
        "That single camera can show either your physical webcam or one selected display. StageSwap changes between them without starting Zoom’s screen-sharing mode." => ("Cette caméra unique peut afficher votre webcam physique ou un écran sélectionné. StageSwap passe de l’un à l’autre sans démarrer le partage d’écran de Zoom.", "Esa única cámara puede mostrar la cámara web física o una pantalla seleccionada. StageSwap cambia entre ambas sin iniciar el modo de compartir pantalla de Zoom."),
        "Private and silent" => ("Privé et silencieux", "Privado y silencioso"),
        "Camera and screen frames stay on this computer. StageSwap does not record them, upload them, or send audio." => ("Les images de la caméra et de l’écran restent sur cet ordinateur. StageSwap ne les enregistre pas, ne les téléverse pas et n’envoie aucun son.", "Las imágenes de la cámara y la pantalla permanecen en este equipo. StageSwap no las graba, no las sube ni envía audio."),
        "First, save a reference" => ("Commencez par enregistrer une référence", "Primero, guarda una referencia"),
        "The reference is a picture of the display when you want people in Zoom to see your webcam. It is usually an idle slide, holding image, or desktop background." => ("La référence est une image de l’écran lorsque vous souhaitez que les participants à Zoom voient votre webcam. Il s’agit généralement d’une diapositive d’attente, d’une image fixe ou de l’arrière-plan du bureau.", "La referencia es una imagen de la pantalla cuando quieres que las personas de Zoom vean tu cámara web. Suele ser una diapositiva de espera, una imagen fija o el fondo del escritorio."),
        "StageSwap watches the picture" => ("StageSwap surveille l’image", "StageSwap vigila la imagen"),
        "It compares the live display with the reference four times per second. It looks only at visual similarity; it does not read slide titles, app names, or text." => ("Il compare l’écran en direct à la référence quatre fois par seconde. Il examine uniquement la ressemblance visuelle ; il ne lit ni les titres des diapositives, ni les noms des applications, ni le texte.", "Compara la pantalla en directo con la referencia cuatro veces por segundo. Solo analiza la similitud visual; no lee títulos de diapositivas, nombres de aplicaciones ni texto."),
        "The match chooses the view" => ("La correspondance choisit la vue", "La coincidencia elige la vista"),
        "A matching display selects the webcam. A changed display selects the screen. When the reference returns, StageSwap switches back to the webcam." => ("Un écran correspondant sélectionne la webcam. Un écran modifié sélectionne l’écran. Lorsque la référence réapparaît, StageSwap revient à la webcam.", "Una pantalla que coincide selecciona la cámara web. Una pantalla que cambia selecciona la pantalla. Cuando vuelve la referencia, StageSwap regresa a la cámara web."),
        "Follow the full video path" => ("Suivez le parcours vidéo complet", "Sigue todo el recorrido del vídeo"),
        "The four previews update live before and during a Zoom call. They let you confirm both selected inputs, the saved reference, and the final result." => ("Les quatre aperçus se mettent à jour en direct avant et pendant un appel Zoom. Ils permettent de vérifier les deux entrées sélectionnées, la référence enregistrée et le résultat final.", "Las cuatro vistas previas se actualizan en directo antes y durante una llamada de Zoom. Permiten comprobar las dos entradas seleccionadas, la referencia guardada y el resultado final."),
        "Green outline: active input" => ("Contour vert : entrée active", "Contorno verde: entrada activa"),
        "The green outline marks the webcam or screen currently feeding the result." => ("Le contour vert indique la webcam ou l’écran qui alimente actuellement le résultat.", "El contorno verde marca la cámara web o la pantalla que alimenta el resultado en ese momento."),
        "Red outline: Zoom output" => ("Contour rouge : sortie Zoom", "Contorno rojo: salida de Zoom"),
        "The red Output outline marks the feed sent to Zoom. FPS shows the live frame rate." => ("Le contour rouge de Sortie indique le flux envoyé à Zoom. FPS affiche la fréquence d’images en direct.", "El contorno rojo de Salida marca la señal enviada a Zoom. FPS muestra la frecuencia de fotogramas en directo."),
        "Check the three components" => ("Vérifiez les trois composants", "Comprueba los tres componentes"),
        "Webcam, Screen, and Output show whether each part is ready. Check this area first if a preview is missing or Zoom shows no picture." => ("Webcam, Écran et Sortie indiquent si chaque élément est prêt. Vérifiez d’abord cette zone si un aperçu manque ou si Zoom n’affiche aucune image.", "Cámara web, Pantalla y Salida indican si cada parte está lista. Comprueba primero esta zona si falta una vista previa o Zoom no muestra imagen."),
        "Check the current decision" => ("Vérifiez la décision actuelle", "Comprueba la decisión actual"),
        "Detection reports whether the live screen matches the reference. Screen mix shows Webcam only, Screen only, or Crossfading while StageSwap moves between them." => ("Détection indique si l’écran en direct correspond à la référence. Mixage écran affiche Webcam uniquement, Écran uniquement ou Fondu enchaîné pendant le passage de l’un à l’autre.", "Detección indica si la pantalla en directo coincide con la referencia. Mezcla de pantalla muestra Solo cámara web, Solo pantalla o Fundido cruzado mientras StageSwap cambia entre ellas."),
        "Start or stop the output" => ("Démarrez ou arrêtez la sortie", "Inicia o detén la salida"),
        "Start automation makes the selected mode live in Zoom. Stop automation keeps the StageSwap camera available but replaces its picture with the black StageSwap off screen." => ("Démarrer l’automatisation diffuse le mode sélectionné dans Zoom. Arrêter l’automatisation laisse la caméra StageSwap disponible, mais remplace son image par l’écran noir d’arrêt de StageSwap.", "Iniciar automatización activa el modo seleccionado en Zoom. Detener automatización mantiene disponible la cámara StageSwap, pero sustituye su imagen por la pantalla negra de StageSwap desactivado."),
        "Choose how StageSwap decides" => ("Choisissez comment StageSwap décide", "Elige cómo decide StageSwap"),
        "Automatic, Webcam, and Screen are output modes. A manual Webcam or Screen choice stays selected until you choose another mode." => ("Automatique, Webcam et Écran sont des modes de sortie. Un choix manuel de Webcam ou Écran reste sélectionné jusqu’à ce que vous choisissiez un autre mode.", "Automático, Cámara web y Pantalla son modos de salida. Una selección manual de Cámara web o Pantalla permanece activa hasta que elijas otro modo."),
        "Capture the idle view" => ("Capturez la vue d’attente", "Captura la vista de espera"),
        "Show the normal idle view on your selected display, then choose Capture reference. StageSwap saves that exact view as the picture Automatic mode should recognize." => ("Affichez la vue d’attente habituelle sur l’écran sélectionné, puis choisissez Capturer la référence. StageSwap enregistre cette vue exacte comme image à reconnaître par le mode Automatique.", "Muestra la vista de espera habitual en la pantalla seleccionada y elige Capturar referencia. StageSwap guarda esa vista exacta como la imagen que debe reconocer el modo Automático."),
        "Find the correct display" => ("Trouvez le bon écran", "Encuentra la pantalla correcta"),
        "Rescan screens searches connected displays for the saved reference. It helps StageSwap find the right display; it does not restart screen capture." => ("Rechercher les écrans cherche la référence enregistrée sur les écrans connectés. Cette action aide StageSwap à trouver le bon écran ; elle ne redémarre pas la capture.", "Buscar pantallas busca la referencia guardada en las pantallas conectadas. Ayuda a StageSwap a encontrar la pantalla correcta; no reinicia la captura."),
        "Adjust the setup in Settings" => ("Ajustez la configuration dans Paramètres", "Ajusta la configuración en Ajustes"),
        "Use Settings to choose the webcam and display, adjust matching, control startup behavior, or open recovery tools." => ("Utilisez Paramètres pour choisir la webcam et l’écran, ajuster la correspondance, contrôler le démarrage ou ouvrir les outils de récupération.", "Usa Ajustes para elegir la cámara web y la pantalla, ajustar la coincidencia, controlar el inicio o abrir las herramientas de recuperación."),
        "Complete the four checks below" => ("Effectuez les quatre vérifications ci-dessous", "Completa las cuatro comprobaciones"),
        "StageSwap is ready once its two inputs are selected, a reference is saved, and StageSwap is chosen as the camera in Zoom." => ("StageSwap est prêt lorsque ses deux entrées sont sélectionnées, qu’une référence est enregistrée et que StageSwap est choisi comme caméra dans Zoom.", "StageSwap está listo cuando se han seleccionado las dos entradas, se ha guardado una referencia y se ha elegido StageSwap como cámara en Zoom."),
        "Return whenever you need help" => ("Revenez quand vous avez besoin d’aide", "Vuelve cuando necesites ayuda"),
        "You can reopen this tutorial from General Settings at any time. The tutorial never changes your devices, reference, mode, or automation state." => ("Vous pouvez rouvrir ce tutoriel depuis les Paramètres généraux à tout moment. Le tutoriel ne modifie jamais vos appareils, votre référence, votre mode ni l’état de l’automatisation.", "Puedes volver a abrir este tutorial desde Ajustes generales en cualquier momento. El tutorial nunca cambia tus dispositivos, la referencia, el modo ni el estado de la automatización."),
        "One camera in Zoom" => ("Une caméra dans Zoom", "Una cámara en Zoom"),
        "Choose StageSwap from Zoom’s camera list." => ("Choisissez StageSwap dans la liste des caméras de Zoom.", "Elige StageSwap en la lista de cámaras de Zoom."),
        "Two possible sources" => ("Deux sources possibles", "Dos fuentes posibles"),
        "StageSwap supplies the webcam or selected display." => ("StageSwap fournit la webcam ou l’écran sélectionné.", "StageSwap proporciona la cámara web o la pantalla seleccionada."),
        "The saved display image that means “show my webcam.”" => ("L’image d’écran enregistrée qui signifie « afficher ma webcam ».", "La imagen de pantalla guardada que significa «mostrar mi cámara web»."),
        "Reference matches" => ("La référence correspond", "La referencia coincide"),
        "People in Zoom see the webcam." => ("Les participants à Zoom voient la webcam.", "Las personas de Zoom ven la cámara web."),
        "Display changes" => ("L’écran change", "La pantalla cambia"),
        "People in Zoom see the selected screen." => ("Les participants à Zoom voient l’écran sélectionné.", "Las personas de Zoom ven la pantalla seleccionada."),
        "The picture from the selected physical camera." => ("L’image de la caméra physique sélectionnée.", "La imagen de la cámara física seleccionada."),
        "The live picture from the selected display." => ("L’image en direct de l’écran sélectionné.", "La imagen en directo de la pantalla seleccionada."),
        "The saved picture used by Automatic mode." => ("L’image enregistrée utilisée par le mode Automatique.", "La imagen guardada que utiliza el modo Automático."),
        "Exactly what people in Zoom receive." => ("Exactement ce que reçoivent les participants à Zoom.", "Exactamente lo que reciben las personas de Zoom."),
        "Green" => ("Vert", "Verde"),
        "Ready, matching, or currently selected." => ("Prêt, correspondant ou actuellement sélectionné.", "Listo, coincidente o seleccionado actualmente."),
        "Amber" => ("Orange", "Ámbar"),
        "Starting, waiting, not matching, or changing." => ("En cours de démarrage, en attente, sans correspondance ou en transition.", "Iniciando, esperando, sin coincidencia o cambiando."),
        "Red" => ("Rouge", "Rojo"),
        "Unavailable, failed, or missing a reference." => ("Indisponible, en échec ou référence manquante.", "No disponible, con error o sin referencia."),
        "Uses the reference to choose webcam or screen." => ("Utilise la référence pour choisir la webcam ou l’écran.", "Utiliza la referencia para elegir la cámara web o la pantalla."),
        "Keeps the webcam visible." => ("Maintient la webcam visible.", "Mantiene visible la cámara web."),
        "Keeps the selected display visible." => ("Maintient l’écran sélectionné visible.", "Mantiene visible la pantalla seleccionada."),
        "Save the display’s current view." => ("Enregistre la vue actuelle de l’écran.", "Guarda la vista actual de la pantalla."),
        "Find the display containing the reference." => ("Trouve l’écran contenant la référence.", "Encuentra la pantalla que contiene la referencia."),
        "Choose devices, matching, behavior, and recovery." => ("Choisissez les appareils, la correspondance, le comportement et la récupération.", "Elige dispositivos, coincidencia, comportamiento y recuperación."),
        "Choose inputs" => ("Choisissez les entrées", "Elige las entradas"),
        "Select the webcam and display in Settings." => ("Sélectionnez la webcam et l’écran dans Paramètres.", "Selecciona la cámara web y la pantalla en Ajustes."),
        "Save the idle view" => ("Enregistrez la vue d’attente", "Guarda la vista de espera"),
        "Show it on the display and capture the reference." => ("Affichez-la sur l’écran et capturez la référence.", "Muéstrala en la pantalla y captura la referencia."),
        "Choose StageSwap" => ("Choisissez StageSwap", "Elige StageSwap"),
        "Select it as the camera in Zoom." => ("Sélectionnez-le comme caméra dans Zoom.", "Selecciónalo como cámara en Zoom."),
        "Go live" => ("Passez en direct", "Empieza la emisión"),
        "Return to the dashboard and start automation." => ("Revenez au tableau de bord et démarrez l’automatisation.", "Vuelve al panel e inicia la automatización."),

        // Tray and Windows notifications.
        "Open StageSwap" => ("Ouvrir StageSwap", "Abrir StageSwap"),
        "Webcam only" => ("Webcam uniquement", "Solo cámara web"),
        "Screen only" => ("Écran uniquement", "Solo pantalla"),
        "Exit" => ("Quitter", "Salir"),
        "StageSwap needs attention" => ("StageSwap nécessite votre attention", "StageSwap requiere atención"),
        "StageSwap installation failed" => ("Échec de l’installation de StageSwap", "Error al instalar StageSwap"),
        "StageSwap is already running" => ("StageSwap est déjà en cours d’exécution", "StageSwap ya está en ejecución"),
        "StageSwap could not start" => ("Impossible de démarrer StageSwap", "No se pudo iniciar StageSwap"),
        "StageSwap could not start its local control service" => ("StageSwap n’a pas pu démarrer son service de contrôle local", "StageSwap no pudo iniciar su servicio de control local"),
        "StageSwap deployment failed" => ("Échec du déploiement de StageSwap", "Error en el despliegue de StageSwap"),
        "Import reference image" => ("Importer une image de référence", "Importar imagen de referencia"),
        "Export diagnostic logs" => ("Exporter les journaux de diagnostic", "Exportar registros de diagnóstico"),
        "Image files" => ("Fichiers image", "Archivos de imagen"),
        "PNG files" => ("Fichiers PNG", "Archivos PNG"),
        "JPEG files" => ("Fichiers JPEG", "Archivos JPEG"),
        "BMP files" => ("Fichiers BMP", "Archivos BMP"),
        "JSON Lines files" => ("Fichiers JSON Lines", "Archivos JSON Lines"),
        "All files" => ("Tous les fichiers", "Todos los archivos"),

        // Portable install and update prompts.
        "Replace the installed StageSwap?" => ("Remplacer la version installée de StageSwap ?", "¿Sustituir la versión instalada de StageSwap?"),
        "Update installed StageSwap" => ("Mettre à jour StageSwap", "Actualizar StageSwap instalado"),
        "Open installed StageSwap" => ("Ouvrir StageSwap installé", "Abrir StageSwap instalado"),
        "Install StageSwap for this user?" => ("Installer StageSwap pour cet utilisateur ?", "¿Instalar StageSwap para este usuario?"),
        "Installation keeps the app at a stable per-user path and creates Start Menu and Desktop shortcuts. The virtual-camera component still requires administrator approval." => ("L’installation conserve l’application dans un emplacement stable propre à l’utilisateur et crée des raccourcis dans le menu Démarrer et sur le Bureau. Le composant de caméra virtuelle nécessite toujours l’autorisation d’un administrateur.", "La instalación mantiene la aplicación en una ruta estable para el usuario y crea accesos directos en el menú Inicio y en el Escritorio. El componente de cámara virtual sigue necesitando aprobación de administrador."),
        "Install StageSwap\nRecommended for startup and upgrades" => ("Installer StageSwap\nRecommandé pour le démarrage et les mises à jour", "Instalar StageSwap\nRecomendado para el inicio y las actualizaciones"),
        "Run once\nDo not copy this executable" => ("Exécuter une fois\nNe pas copier cet exécutable", "Ejecutar una vez\nNo copiar este ejecutable"),
        "This replaces the installed app with an older version." => ("Cette opération remplace l’application installée par une version antérieure.", "Esto sustituye la aplicación instalada por una versión anterior."),
        "This replaces it with a different build of the same version." => ("Cette opération la remplace par une autre compilation de la même version.", "Esto la sustituye por otra compilación de la misma versión."),
        "The running app will close gracefully before replacement." => ("L’application en cours d’exécution se fermera correctement avant son remplacement.", "La aplicación en ejecución se cerrará correctamente antes de sustituirse.")
        ,"Installed version: {0}\nCandidate version: {1}\n\n{2}" => ("Version installée : {0}\nVersion candidate : {1}\n\n{2}", "Versión instalada: {0}\nVersión candidata: {1}\n\n{2}")
    }))
}

pub fn format_text(locale: Locale, source: &str, arguments: &[&str]) -> String {
    let mut result = text(locale, source).into_owned();
    for (index, value) in arguments.iter().enumerate() {
        result = result.replace(&format!("{{{index}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_tags_accept_language_families_and_normalize() {
        assert_eq!(Locale::from_tag("fr-CA"), Some(Locale::French));
        assert_eq!(Locale::from_tag("es_MX"), Some(Locale::Spanish));
        assert_eq!(Locale::from_tag("EN-gb"), Some(Locale::English));
        assert_eq!(Locale::from_tag("de-DE"), None);
        assert_eq!(Locale::Spanish.tag(), "es");
    }

    #[test]
    fn saved_locale_wins_then_system_then_english() {
        assert_eq!(
            Locale::resolve(Some("fr-FR"), Some("es-ES")),
            Locale::French
        );
        assert_eq!(Locale::resolve(None, Some("es-MX")), Locale::Spanish);
        assert_eq!(
            Locale::resolve(Some("unknown"), Some("fr-CA")),
            Locale::French
        );
        assert_eq!(Locale::resolve(None, Some("de-DE")), Locale::English);
    }

    #[test]
    fn known_text_is_translated_and_unknown_text_is_preserved() {
        assert_eq!(text(Locale::French, "Settings"), "Paramètres");
        assert_eq!(text(Locale::Spanish, "Settings"), "Ajustes");
        assert_eq!(
            text(Locale::French, "Device supplied name"),
            "Device supplied name"
        );
    }

    #[test]
    fn formatted_text_reorders_safe_placeholders() {
        assert_eq!(
            format_text(Locale::French, "{0} — {1}", &["98 %", "Équilibrée"]),
            "98 % — Équilibrée"
        );
    }
}
